# Source quality commands

The checker reads only Git-tracked source files with the approved extensions in
`config.json`. The generated `web/public/mockServiceWorker.js` is the only
declared exclusion.

Run the commands from the repository root:

```text
tools/source-structure-check/baseline
tools/source-structure-check/check --all
tools/source-structure-check/check --staged
tools/source-structure-check/require-zero
```

`baseline` creates the ratchet file once, or rewrites it only after every
diagnostic identity and metric has stayed the same or decreased. `check` never
mutates state. `require-zero` succeeds only with no diagnostics, removes the
ratchet, and records the irreversible `zero` state. Parsing errors always fail
before a baseline can be written.
