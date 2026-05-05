# New Agent

I want to completely rewrite and redesign the current forge agent. The goal is to build an agent that can run on Temporal.

The last few weeks, I worked on a different project that, called "agent os" or AOS. It's a very different system, but the core idea is that it is all event sourced. You can read the specs here: refs/aos-spec/

I copied the relevant code over from the old repo:
- refs/aos-agent/
- refs/aos-cli/ (see esp. refs/aos-cli/src/chat/)

That agent is _conceptually_ further along than the forge agent.

Because we want to start from scratch, I reset the forge agent crate (crates/forge-agent/). The old version, that we used to have, currently is here: refs/forge-agent/ there is some good stuff in there too. Note that crates/forge-attractor/ is currently not buildig due to that, which is fine for now. We will later have to redesign attractor too.

## Temporal

With agent os, we tried to basically build somethign similar to Temporal. The better approach is to just go with temporal.

So, I want the new agent to be desgined _for_ running on temporal. We do need to decide if we can make the core temporal agnostic or if we should just build it deeply into teporal from the get-go.

## Codex

Codex is a an advanced agent written in Rust. You can find the code here: /Users/lukas/dev/tmp/codex

Another option is to link/vendor in codex and make the backend work better with Temporal instead of building an agent from scratch.


## CLI

The cli should work similar to the codex cli or the cli we've built here: refs/aos-cli/src/chat (which was also codex inspired)

