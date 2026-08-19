#!/usr/bin/env node

import { spawnSync } from 'node:child_process';
import {
  appendFile,
  mkdir,
  readFile,
  realpath,
  rename,
  stat,
  unlink,
  writeFile,
} from 'node:fs/promises';
import path from 'node:path';
import process from 'node:process';
import { text } from 'node:stream/consumers';
import { parseArgs } from 'node:util';

const TEAM_FILE = '.agents/orchestration.json';
const LOCAL_FILE = '.agents/orchestration.local.json';
const LOCAL_EXCLUDE_PATTERN = '/.agents/orchestration.local.json';
const ROLE_FIELDS = new Set(['agent_type', 'agent_tool', 'model', 'effort']);
const ROLE_NAME = /^[a-z][a-z0-9-]{0,63}$/;

function usage() {
  return `Usage:
  orchestration-config.mjs read --project-root PATH
  orchestration-config.mjs write <team|local> --project-root PATH

Run with Node.js 18.3+, Bun, or Deno 2. Write commands read one JSON document from stdin.`;
}

function fail(message) {
  throw new Error(message);
}

function parseCli() {
  const { values, positionals } = parseArgs({
    options: {
      help: { type: 'boolean', short: 'h' },
      'project-root': { type: 'string' },
    },
    allowPositionals: true,
    strict: true,
  });

  if (values.help) return { help: true };

  const [command, target, ...extra] = positionals;
  if (!command || extra.length > 0) fail(usage());
  if (!values['project-root']) fail('--project-root is required.');

  if (command === 'read' && target === undefined) {
    return { command, projectRoot: values['project-root'] };
  }

  if (command === 'write' && ['team', 'local'].includes(target)) {
    return { command, target, projectRoot: values['project-root'] };
  }

  fail(usage());
}

async function resolveProjectRoot(input) {
  const requestedPath = path.resolve(input);

  let details;
  try {
    details = await stat(requestedPath);
  } catch {
    fail(`Project root does not exist: ${requestedPath}`);
  }

  if (!details.isDirectory()) fail(`Project root is not a directory: ${requestedPath}`);
  return realpath(requestedPath);
}

function configPaths(projectRoot) {
  return {
    team: path.join(projectRoot, TEAM_FILE),
    local: path.join(projectRoot, LOCAL_FILE),
  };
}

function assertObject(value, label) {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    fail(`${label} must be a JSON object keyed by role name.`);
  }
}

function validateRoles(roles, label) {
  assertObject(roles, label);

  for (const [roleName, role] of Object.entries(roles)) {
    if (!ROLE_NAME.test(roleName)) fail(`${label} contains invalid role name: ${roleName}`);
    assertObject(role, `${label}.${roleName}`);

    const unknown = Object.keys(role).filter((field) => !ROLE_FIELDS.has(field));
    if (unknown.length > 0) {
      fail(`${label}.${roleName} contains unknown field(s): ${unknown.join(', ')}`);
    }

    for (const [field, value] of Object.entries(role)) {
      if (typeof value !== 'string' || value.trim() === '') {
        fail(`${label}.${roleName}.${field} must be a non-empty string.`);
      }
    }
  }

  return roles;
}

async function readRoles(filePath, label) {
  let contents;
  try {
    contents = await readFile(filePath, 'utf8');
  } catch (error) {
    if (error.code === 'ENOENT') return null;
    throw error;
  }

  try {
    return validateRoles(JSON.parse(contents), label);
  } catch (error) {
    if (error instanceof SyntaxError) fail(`${label} is not valid JSON: ${error.message}`);
    throw error;
  }
}

function mergeRoles(team, local) {
  const effective = {};

  for (const [roleName, role] of Object.entries(team ?? {})) {
    effective[roleName] = { ...role };
  }

  for (const [roleName, role] of Object.entries(local ?? {})) {
    effective[roleName] = { ...(effective[roleName] ?? {}), ...role };
  }

  return effective;
}

async function loadConfiguration(paths) {
  const team = await readRoles(paths.team, 'team configuration');
  const local = await readRoles(paths.local, 'local configuration');
  return { team, local, effective: mergeRoles(team, local) };
}

async function readStdinJson() {
  const contents = (await text(process.stdin)).trim();
  if (!contents) fail('Expected a JSON document on stdin.');

  try {
    return JSON.parse(contents);
  } catch (error) {
    fail(`stdin is not valid JSON: ${error.message}`);
  }
}

async function atomicWrite(filePath, value, mode) {
  await mkdir(path.dirname(filePath), { recursive: true });
  const temporaryPath = `${filePath}.${process.pid}.tmp`;

  try {
    await writeFile(temporaryPath, `${JSON.stringify(value, null, 2)}\n`, { mode });
    await rename(temporaryPath, filePath);
  } finally {
    await unlink(temporaryPath).catch((error) => {
      if (error.code !== 'ENOENT') throw error;
    });
  }
}

function runGit(projectRoot, args) {
  const gitArguments = ['-C', projectRoot, ...args];

  if (typeof globalThis.Deno !== 'undefined') {
    const result = new globalThis.Deno.Command('git', {
      args: gitArguments,
      stdin: 'null',
      stdout: 'piped',
      stderr: 'piped',
    }).outputSync();
    const decoder = new TextDecoder();

    return { status: result.code, stdout: decoder.decode(result.stdout) };
  }

  return spawnSync('git', gitArguments, {
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
  });
}

async function ensureLocalExclude(projectRoot) {
  const result = runGit(projectRoot, ['rev-parse', '--git-path', 'info/exclude']);
  if (result.status !== 0 || !result.stdout.trim()) {
    return { added: false, reason: 'Project is not a Git worktree.' };
  }

  const reportedPath = result.stdout.trim();
  const excludePath = path.isAbsolute(reportedPath)
    ? reportedPath
    : path.resolve(projectRoot, reportedPath);

  let contents = '';
  try {
    contents = await readFile(excludePath, 'utf8');
  } catch (error) {
    if (error.code !== 'ENOENT') throw error;
  }

  if (!contents.split(/\r?\n/).includes(LOCAL_EXCLUDE_PATTERN)) {
    await mkdir(path.dirname(excludePath), { recursive: true });
    const prefix = contents === '' || contents.endsWith('\n') ? '' : '\n';
    await appendFile(excludePath, `${prefix}${LOCAL_EXCLUDE_PATTERN}\n`, 'utf8');
  }

  return { added: true, exclude_path: excludePath };
}

function printJson(value) {
  process.stdout.write(`${JSON.stringify(value, null, 2)}\n`);
}

async function main() {
  const cli = parseCli();
  if (cli.help) {
    process.stdout.write(`${usage()}\n`);
    return;
  }

  const projectRoot = await resolveProjectRoot(cli.projectRoot);
  const paths = configPaths(projectRoot);

  if (cli.command === 'read') {
    const configuration = await loadConfiguration(paths);
    printJson({ project_root: projectRoot, files: paths, ...configuration });
    return;
  }

  const proposed = validateRoles(await readStdinJson(), `${cli.target} configuration`);
  if (cli.target === 'team') {
    await readRoles(paths.local, 'local configuration');
  } else {
    await readRoles(paths.team, 'team configuration');
  }

  const destination = cli.target === 'team' ? paths.team : paths.local;
  const result = { written: destination, scope: cli.target };

  if (cli.target === 'local') {
    result.git_exclude = await ensureLocalExclude(projectRoot);
  }

  await atomicWrite(destination, proposed, cli.target === 'team' ? 0o644 : 0o600);
  printJson(result);
}

main().catch((error) => {
  process.stderr.write(`orchestration-config: ${error.message}\n`);
  process.exitCode = 1;
});
