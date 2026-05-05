# Factory Control Plane Specification (Ideation)

This document defines the broad shape of the factory layer above Attractor.
It is intentionally conceptual and outcome-oriented, not an API or interface contract.

It extends, and does not replace:
- `00-vision.md` for principles and vocabulary
- `02-coding-agent-loop-spec.md` for stage-level coding execution
- `03-attractor-spec.md` for DOT pipeline orchestration
- `04-cxdb-integration-spec.md` for durable runtime persistence/query boundaries

---

## 1. Why this layer exists

Attractor is the inner execution loop: it runs a graph well.
A software factory also needs an outer convergence loop: it decides what graph to run next, why, and when to stop.

Without that outer layer, we can execute workflows, but we cannot reliably compound correctness across many runs, scenarios, and changing constraints.

---

## 2. Core stance

### 2.1 Inner/outer split

- Attractor DOT graphs are the **inner plan artifact** for one run (or one bounded supervision stack).
- The factory control plane is the **outer policy and convergence system** across runs.

Reason:
- Graphs are excellent at deterministic local flow.
- Factory behavior requires global choices: priorities, budgets, holdouts, risk posture, and release decisions over time.

### 2.2 Specs and scenarios remain the source of truth

The control plane should treat code and graphs as generated means, not the primary intent.

Reason:
- Product intent and acceptance criteria should survive model, framework, and implementation churn.
- This is the only stable way to avoid local optimization drift.

---

## 3. What the factory layer should optimize for

### 3.1 Convergence, not single-run success

The control plane should optimize for sustained satisfaction across scenario packs and holdouts, not one green run.

Reason:
- Single runs are noisy and easy to game.
- Persistent satisfaction better tracks real user outcomes.

### 3.2 Scenario sovereignty

The factory should treat scenario packs (especially holdouts) as a protected contract.

Reason:
- If the system can freely rewrite its own judgeable target, reward hacking becomes the default.
- Externalized scenarios preserve pressure toward true behavior.

### 3.3 Validation economics

The factory should actively manage cost, latency, and coverage tradeoffs.

Reason:
- Unlimited loops are not a strategy.
- The system must know when to spend tokens for confidence and when to stop.

### 3.4 Portfolio scheduling

The factory should schedule work as a portfolio: bug risk, feature impact, confidence debt, and infra constraints.

Reason:
- A software system is not one queue.
- Highest-leverage work is often not the most recently requested work.

### 3.5 Robustness against reward hacking

The factory should use cross-checks, varied judges, and adversarial scenarios as first-class control signals.

Reason:
- Any single evaluator can be overfit.
- Diversity of evidence is required for trustworthy autonomy.

### 3.6 Memory with accountability

The factory should accumulate durable run memory: what changed, why it was accepted, what later regressed.

Reason:
- Learning across runs is the mechanism for compounding correctness.
- Auditability is required for trust, debugging, and governance.

### 3.7 Policy and risk governance

The factory should enforce non-negotiable policy boundaries (security, legal, safety, reliability) before declaring success.

Reason:
- User satisfaction alone is not sufficient if policy constraints are violated.
- Guardrails must be explicit and machine-enforced.

### 3.8 Human role as intent and policy author

The factory should minimize human code-writing and line-by-line review, while maximizing human control over intent, policy, and escalation.

Reason:
- This preserves speed gains from automation without removing accountability.

---

## 4. How this relates to Attractor

Attractor remains the runtime for executing a chosen plan graph.
The control plane may select a graph template, compose graphs, or synthesize graphs, but DOT remains an inner-loop execution representation.

Reason:
- This preserves a clean separation:
  - control plane decides
  - Attractor executes
- Clear boundaries make behavior easier to reason about, test, and evolve.

---

## 5. Expected outcomes

When this layer is healthy, Forge should show:

- Higher satisfaction stability across repeated runs
- Lower cost-to-satisfy over time (learning effects)
- Faster time-to-satisfy after spec/scenario changes
- Better incident explainability through evidence-rich lineage
- Fewer regressions escaping holdout and policy gates

---

## 6. Non-goals for this phase

- Defining concrete interfaces, RPC schemas, or crate APIs
- Locking in one graph synthesis strategy
- Designing UI surfaces in detail
- Defining distributed scheduler protocols

This document is intentionally a directional blueprint for product and architecture decisions, not an implementation contract.

---

## 7. Open strategic questions

- When should the control plane synthesize a new graph versus reuse known templates?
- How should confidence be represented when judges disagree?
- What stop rules balance speed, confidence, and cost for different product domains?
- Which failures require automatic escalation to human policy owners?

These questions are deliberately left open for follow-on specs once runtime migration and query/control surfaces are stabilized.
