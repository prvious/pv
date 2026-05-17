# pv Rewrite Context

The pv rewrite context defines the language used by the Laravel-first local control-plane work. These terms are product/domain terms, not package names.

## Language

**Active Rewrite Workspace**:
The repository root where new pv rewrite code is developed.
_Avoid_: root prototype, current prototype

**Legacy Prototype**:
The old implementation kept only as reference material.
_Avoid_: active code, compatibility layer

**Project Contract**:
A human-authored `pv.yml` file that declares what a project asks pv to manage.
_Avoid_: inferred config, generated state

**Desired State**:
The machine-owned record of what the user requested pv to make true.
_Avoid_: status, runtime facts

**Observed Status**:
The machine-owned record of what pv found or did while reconciling desired state.
_Avoid_: desired state, source of truth

**Store**:
The machine-owned authority for desired state, observed status, and pv metadata.
_Avoid_: project config, registry JSON

**Controller**:
The resource owner that reconciles desired state into observed status.
_Avoid_: command handler, supervisor

**Resource**:
A runtime, tool, backing service, project, or gateway capability managed by pv.
_Avoid_: generic service

**Capability**:
A precise behavior a resource exposes, such as installable, runnable, stateful, or env provider.
_Avoid_: fake generic interface

**Supervisor**:
The process runner that starts, stops, checks, and reports managed processes without resource knowledge.
_Avoid_: resource manager, service controller

**Daemon**:
The long-running reconciler that dispatches controllers and records observed status.
_Avoid_: supervisor, command runner

**Gateway**:
The managed HTTPS `.test` routing surface for linked projects.
_Avoid_: user service, generic proxy

**Managed Env Entry**:
An `.env` key written by pv from declared project contract templates and labeled as pv-managed.
_Avoid_: auto-detected env, Laravel smart var

**Setup Command**:
An ordered shell command string declared by the project contract and run during link.
_Avoid_: hidden setup step, generic command runner

**Post-MVP Backlog**:
The list of intentionally deferred capabilities with reasons and reconsideration triggers.
_Avoid_: wishlist, hidden scope

## Relationships

- A **Project Contract** can produce one durable project **Desired State** record.
- A **Controller** reads **Desired State** and writes **Observed Status** for one resource family.
- The **Daemon** coordinates **Controllers**; the **Supervisor** only runs process definitions.
- The **Store** owns **Desired State** and **Observed Status**; a **Project Contract** remains human-authored.
- **Managed Env Entries** are rendered from a **Project Contract** and resource capabilities, never inferred from existing `.env` values.
- A **Post-MVP Backlog** item is not an MVP dependency until it is deliberately promoted through new planning work.

## Example Dialogue

> **Dev:** "Can `pv link` read `.env` and bind Redis if it sees `CACHE_STORE=redis`?"
> **Domain expert:** "No. Redis must be declared in the **Project Contract**. `pv link` records **Desired State**, renders only declared **Managed Env Entries**, and the **Daemon** asks the Redis **Controller** to reconcile."

## Flagged Ambiguities

- "service" was used for every managed thing; resolved term is **Resource**, with **Capability** names for behavior differences.
- "state" was used for both requests and reality; resolved terms are **Desired State** and **Observed Status**.
- "setup" was used for hidden Laravel automation and user-authored commands; resolved term is **Setup Command**, and only declared commands are run.
