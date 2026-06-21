# PV

PV manages local development environments for PHP projects.

## Language

**Project**:
A local application directory managed by PV.
_Avoid_: Site, app

**Project hostname**:
The `.test` hostname PV assigns to a **Project**.
_Avoid_: Domain, site URL

**Project config**:
The `pv.yml` or `pv.yaml` file that declares a **Project's** PV-specific requirements.
_Avoid_: Manifest

**Gateway**:
The PV-managed local web entry point that routes **Project hostnames** to **Projects**.
_Avoid_: Main FrankenPHP

**Managed Resource**:
A tool, runtime, or backing service installed and managed by PV.
_Avoid_: Tool, service when referring to the whole category

**Managed Resource artifact**:
A PV-owned packaged installable for a **Managed Resource**.
_Avoid_: Upstream binary, local build recipe

**PHP track defaults**:
PV-owned PHP configuration associated with a PHP version track.
_Avoid_: php.ini support, custom Project ini

**Artifact manifest**:
The PV-owned catalog of available **Managed Resource artifacts**.
_Avoid_: Project config, per-archive manifest

**Resource allocation**:
A Project-specific object created inside a shared **Managed Resource**.
_Avoid_: Logical resource, project resource

## Relationships

- A **Project** is the unit a developer links into PV.
- A **Project** has one **Project hostname**.
- A **Project hostname** belongs to exactly one **Project**.
- A **Project** may have one **Project config**.
- A **Project** may have many **Resource allocations**.
- A **Resource allocation** belongs to one **Project** and one **Managed Resource**.
- A **Managed Resource** is installed from one or more **Managed Resource artifacts**.
- **PHP track defaults** belong to a PHP version track and apply consistently to PHP execution for that track.
- An **Artifact manifest** lists **Managed Resource artifacts**.
- The **Gateway** routes **Project hostnames** to **Projects**.

## Example dialogue

> **Dev:** "When I run `pv link`, am I adding this directory as a **Project**?"
> **Domain expert:** "Yes — PV manages that local directory as a **Project**."
> **Dev:** "And the browser address is the **Project hostname**?"
> **Domain expert:** "Yes — PV derives that hostname from the **Project** directory unless told otherwise."
> **Dev:** "Does the **Gateway** decide which **Project** receives that request?"
> **Domain expert:** "Yes — the **Gateway** routes the **Project hostname** to the linked **Project**."
> **Dev:** "If PV creates a database for my **Project**, is that a **Resource allocation**?"
> **Domain expert:** "Yes — it is the **Project's** allocation inside the database **Managed Resource**."
> **Dev:** "If Redis has no upstream macOS binary, is the source tarball the **Managed Resource artifact**?"
> **Domain expert:** "No — the **Managed Resource artifact** is the PV-owned package users install."
> **Dev:** "Does each **Managed Resource artifact** contain its own manifest?"
> **Domain expert:** "No — the **Artifact manifest** is the PV-owned catalog outside the archive."
> **Dev:** "If PHP needs default ini settings, are those **PHP track defaults**?"
> **Domain expert:** "Yes — they belong to the PHP version track, not to an individual **Project config**."

## Flagged ambiguities

- "site" and "app" were considered for the managed directory concept — resolved: use **Project**.
- "domain" was considered for the `.test` address — resolved: use **Project hostname**.
- "main FrankenPHP" was considered for the front routing role — resolved: use **Gateway**.
- "logical resource" and "project resource" were considered for per-Project objects inside shared resources — resolved: use **Resource allocation**.
- "artifact" could mean an upstream binary, source archive, or local build recipe — resolved: use **Managed Resource artifact** for the PV-owned packaged installable.
- "manifest" could mean Project config, a per-archive metadata file, or the artifact catalog — resolved: use **Artifact manifest** only for the PV-owned catalog of available Managed Resource artifacts.
- "php.ini support" could mean Project-specific overrides, user-editable ini files, or PV-owned defaults — resolved: use **PHP track defaults** for PV-owned PHP configuration associated with a PHP version track.
