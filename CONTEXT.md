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

## Flagged ambiguities

- "site" and "app" were considered for the managed directory concept — resolved: use **Project**.
- "domain" was considered for the `.test` address — resolved: use **Project hostname**.
- "main FrankenPHP" was considered for the front routing role — resolved: use **Gateway**.
- "logical resource" and "project resource" were considered for per-Project objects inside shared resources — resolved: use **Resource allocation**.
