# Forge Vision: Software Factories in the Agentic Moment

_This is a rephrasing of Strong DM's ideas outlined [here](https://factory.strongdm.ai/)_

Forge is an opinionated “software factory”: **non-interactive development** where **specs + scenarios** drive agent loops that write code, run harnesses, and converge toward outcomes—*without humans writing or reviewing code as a routine step*.

This document is a product vision and a high-level spec for what Forge is trying to become.

## One-liners

- **Forge builds software the way compilers build binaries:** input artifacts in, validated outputs out.
- **Humans author intent; models do the work.** Humans describe “what” and “why”, not “how”.

## Problem statement

Modern software development still assumes:

- Humans write most code.
- Humans review most code.
- Validation is mostly deterministic (tests pass) and mostly in-repo.

Agentic coding changes the economics and the failure modes:

- Iterative LLM-driven changes can either **compound correctness** (improving with each loop) or **compound error** (drift, regressions, reward hacks).
- Traditional tests are necessary but often insufficient: they can be brittle, incomplete, and can be “gamed” by optimizing for narrow signals.

Forge exists to make “compounding correctness” the default outcome by treating evaluation, environments, and iteration as first-class system design.

## Principles

Forge is organized around a simple compounding loop:

**Seed → Validation harness → Feedback loop.**  
**Tokens are the fuel.**

### Seed (entry point)

Every software system starts with an initial seed. Historically this might be a PRD or spec; in Forge it can be any model-readable artifact that captures intent, including:

- A few sentences of product intent
- A screenshot or screen recording
- An existing codebase

At minimum, a seed SHOULD include: desired behavior, constraints, and what “good” looks like.

### Validation (end-to-end harness)

Forge’s validation harness MUST be end-to-end and as close to real environments as possible, including:

- Customers (realistic usage patterns and expectations)
- Integrations (third-party APIs and their edge cases)
- Economics (latency, cost, rate limits, quotas)

The harness SHOULD be scenario-driven and include **holdout scenarios** that the loop cannot trivially rewrite away.

### Feedback (closed loop)

Forge feeds a sample of outputs (and their evidence) back into inputs: failures, near-misses, costs, and judgments become new constraints and new work items.

This closed loop enables self-correction and is the mechanism by which we aim to **compound correctness** rather than compound error.

The loop runs until holdout scenarios pass **and stay passing**.

### Fuel (apply more tokens)

Validation and feedback are easy to describe; the practice requires creative frontier engineering. For each obstacle, ask:

> How can we convert this problem into a representation the model can understand?

Common “fuel adapters” include:

- Traces (structured logs, spans, timings)
- Screen capture (UI state as pixels/video)
- Conversation transcripts (support tickets, chats, calls)
- Incident replays (record/replay production failures)
- Adversarial use (red-team prompts and misuse cases)
- Agentic simulation (synthetic users and environments)
- Just-in-time surveys (in-product feedback at decision points)
- Customer interviews (qualitative reality checks)
- Price elasticity testing (economic constraints and tradeoffs)

## Techniques

Techniques are practical patterns we return to while building with a software factory. They operationalize the principles above and help the loop converge without human code-writing or traditional review.

### The validation constraint (design premise)

Forge is designed for a world with:

- Zero hand-written code (as a default workflow)
- Zero traditional line-by-line review (as a default workflow)

Therefore, Forge MUST be able to:

- **Grow from cascades of natural-language specifications** (seeds → refined specs → executable intent)
- **Validate automatically without semantic inspection of source**

Operationally, Forge treats code like an ML model snapshot: **opaque weights** whose correctness is inferred from **externally observable behavior**. Internal structure is treated as opaque unless it affects behavior, cost, or policy.

### Digital Twin Universe (DTU)

Clone the externally observable behaviors of critical third-party dependencies.

- Purpose: validate at volumes and rates far exceeding production limits.
- Properties: deterministic, replayable test conditions; fault injection; edge-case coverage.
- Fit: scenarios that depend on SaaS APIs, rate limits, quotas, and failure modes.

### Gene transfusion

Move working patterns between codebases by pointing agents at concrete exemplars.

- Pattern: “Here is a known-good implementation; reproduce the behavior in this new context.”
- Requirement: a high-quality reference + a clear behavioral contract (scenarios/judges).
- Benefit: reduces invention risk; increases consistency across repos and stacks.

### The filesystem (as memory)

Use on-disk state as a practical memory substrate.

- Models can quickly navigate repositories by reading/writing files and building indexes.
- Directories, manifests, and generated summaries become durable context that survives runs.
- Implication: Forge SHOULD prefer explicit artifacts (indexes, run reports, summaries) over “remembering” in prompts.

### Shift work

Separate interactive work (intent formation) from fully specified work (factory execution).

- Human time: write/curate specs, scenarios, constraints, and policies.
- Factory time: execute end-to-end loops without back-and-forth once intent is complete.
- Goal: maximize uninterrupted, unattended convergence.

### Semport

Semantically-aware automated ports, one-time or ongoing.

- Use cases: migrate languages/frameworks, upgrade major versions, refactor architectures.
- Constraint: preserve externally observable behavior (scenarios as the contract).
- Mode: one-shot port or continuous “keep in sync” porting across targets.

### Pyramid summaries

Reversible summarization at multiple zoom levels.

- Maintain compressed context (project, subsystem, file) with the ability to expand back to full detail.
- Use summaries as navigation aids for agents and as auditability aids for humans.
- Forge SHOULD treat summaries as artifacts with provenance (what was summarized, when, and from which sources).

## Goals

Forge MUST:

- Convert **written intent** (specs) into **running software** with minimal human intervention.
- Validate against **end-to-end scenarios** that are harder to rewrite away than unit tests.
- Support **probabilistic evaluation** (“satisfaction”), not only pass/fail gates.
- Run safely and repeatably in **sandboxed environments** (including third-party API “twins”).
- Produce a transparent **audit trail** (what changed, why, evidence it works).

Forge SHOULD:

- Enable extremely fast iteration loops (minutes, not days).
- Make validation **cheat-resistant** (holdouts, invariants, and externalized scenarios).
- Make “scale up the loop” a primary knob (tokens, parallelism, scenario volume).

Forge MAY:

- Provide a UI for scenario authoring, run inspection, and regression analysis.
- Provide a marketplace/ecosystem of twins, scenario packs, and judges.

## Non-goals

Forge is not:

- A general-purpose IDE or code editor.
- A foundation model provider.
- A replacement for all tests; it is a higher-level validation and orchestration layer.
- A “magic autopilot” that ignores product intent, security boundaries, or costs.

## Operating principles

### 1) Hands-off by default

Forge is built around a constraint:

- **Code is not written by humans.**
- **Code is not reviewed by humans.**

Interpretation: humans can still *guide, inspect, and override*, but the default workflow should not require humans to manually implement or approve every line. The factory must be able to run unattended.

### 2) Specs are the source of truth

- Specs describe the product intent, invariants, constraints, and tradeoffs.
- Forge treats specs as durable artifacts that outlive any single model run.

### 3) Scenarios over “tests”

Forge distinguishes:

- **Tests:** deterministic checks stored in-repo (useful, but rewriteable).
- **Scenarios:** end-to-end user stories, ideally stored *outside* the code under test (a “holdout set”), that can be validated by a harness and/or an LLM judge.

Scenarios are the main lever for preventing drift and reward hacking.

### 4) Satisfaction over boolean success

Many outcomes are not well-captured by `green/red`:

- UX quality, explanation quality, correctness under ambiguity, partial failures, etc.

Forge uses **satisfaction** as a primary metric:

- “Across many trajectories and scenario runs, what fraction likely satisfies the user?”

Satisfaction is empirical, measured over time, and is expected to be probabilistic.

### 5) Validate in a Digital Twin Universe (DTU)

Forge assumes we cannot safely or cheaply validate at scale against live third-party services.

Forge therefore prefers:

- **Digital twins:** behavioral clones of external dependencies (APIs, edge cases, observable behaviors).
- High-volume validation against twins: thousands of scenario runs per hour without rate limits, abuse triggers, or real API cost.

## Vocabulary (normative)

- **Spec:** a structured description of intent and constraints. Specs MUST be human-authored and model-readable.
- **Scenario:** an end-to-end “user story” with inputs, environment assumptions, and expected outcomes. Scenarios SHOULD be externalized from the code under test.
- **Harness:** the runner that executes scenarios and collects observations (logs, traces, outputs, screenshots, API calls).
- **Judge:** a deterministic checker and/or LLM-based evaluator that scores a run. Judges SHOULD output explanations and evidence references.
- **Satisfaction:** the aggregated score over scenarios (and sampled trajectories) that estimates “works for users”.
- **Twin:** a simulated dependency with an API surface and behavior model (including failure modes).
- **Loop:** the orchestrated cycle of propose → implement → validate → learn → repeat.

## System shape (proposed)

### Inputs (versioned artifacts)

Forge should treat these as first-class, reviewable, and diffable:

- `specs/`: product and system specs (structured markdown or a small DSL)
- `scenarios/`: scenario packs (YAML/JSON/Markdown + fixtures)
- `twins/`: twin definitions and datasets (record/replay + generative behaviors)
- `forge.toml` (or similar): project configuration (models, budgets, policies)

### Core components

- **Orchestrator:** plans work, schedules loops, enforces budgets/policies.
- **Agent runtime:** executes model-driven coding and refactoring steps.
- **Harness runner:** executes scenarios in isolated environments.
- **Judge(s):** scores and explains outcomes; writes satisfaction reports.
- **Twin runtime:** runs twins locally and deterministically; supports fault injection.
- **Run store:** persists run artifacts (diffs, logs, traces, judgments, costs).

## Core workflow (happy path)

1. Human updates `specs/` and/or `scenarios/` (intent and acceptance criteria).
2. Forge creates a plan: what to change, what scenarios are affected, what to re-run.
3. Agent runtime changes the codebase to satisfy the plan.
4. Harness runs scenarios against twins (and optionally real services under strict limits).
5. Judges score results and compute satisfaction.
6. If satisfaction is below policy thresholds, Forge iterates; otherwise it produces:
   - A changelog-style explanation
   - Evidence links to scenario runs
   - A minimal patchset

## Guardrails against reward hacking

Forge SHOULD support:

- **Holdout scenarios** that agents can’t trivially edit as part of normal loops.
- **Cross-checks** (multiple judges, deterministic invariants, metamorphic tests).
- **Policy constraints** (e.g., “no disabling tests”, “no skipping harness”, “no `return true` shortcuts”).
- **Regression budgets** (reject changes that decrease satisfaction on stable scenario packs).

## Success metrics

Forge should optimize for:

- **Satisfaction** (primary) on a representative scenario set.
- **Time-to-satisfy** (wall clock from spec change → acceptable satisfaction).
- **Cost-to-satisfy** (tokens/$ per satisfaction point or per shipped change).
- **Stability** (satisfaction variance across runs, models, and environments).
- **Auditability** (how quickly a human can understand “why this works”).
