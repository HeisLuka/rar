# CHAPTERA-WIN-UPDATE-UX-01 — first slice

This fresh-main branch introduces only the source-neutral update UI state/intent contract.

It deliberately does not:
- perform network I/O;
- mutate updater journals;
- download payloads;
- install or restart;
- bypass Reader close/save authority.

The desktop Reader will consume this contract in a follow-up slice after the state-machine contract is green.
