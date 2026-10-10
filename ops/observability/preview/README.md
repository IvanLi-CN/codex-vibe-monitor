# Grafana dashboard preview

This stack is the repository-owned preview for the Grafana statistics and classified-case dashboards in `../grafana/dashboards`. It starts the real Grafana, Prometheus and Tempo images, then feeds them changing but explicitly synthetic Prometheus series from `metrics_server.py` and synthetic OTLP waterfalls from `traces.py`.

The fixture is intentionally isolated from the production observability contract:

- Prometheus uses the `synthetic-preview` environment label.
- The fixture exposes no production endpoint, database, token, request payload, account, model, or user data.
- Grafana is anonymous Viewer-only HTTP on the Agent VM port mapping.
- Tempo is tenant cvm, local filesystem, 24h retention and a shared 512MiB tmpfs for every mutable path; this is an isolation limit, not production capacity.
- Prometheus, Tempo and Grafana data are disposable `tmpfs` state; `down` removes the stack and its network.
- Dashboard JSON and Grafana provisioning are mounted from the normal `ops/observability/grafana` tree, so dashboard edits are previewed directly.

## Start through the Agent VM

The repository disables local Docker by default. From the repository root:

```bash
scripts/cvm-grafana-preview up
scripts/cvm-grafana-preview url
```

The `up` command acquires or reuses the current session's Agent VM, syncs the tracked repository into its workspace, provisions the preview stack, and prints the URL. Open the printed URL in a browser. The default dashboard range is the last 15 minutes; the fixture changes continuously so time-series panels remain useful while tuning layouts or PromQL.

Useful lifecycle commands:

```bash
scripts/cvm-grafana-preview status
scripts/cvm-grafana-preview logs grafana
scripts/cvm-grafana-preview down
```

Use `scripts/cvm-grafana-preview up --port 3102` when the default guest port is occupied. A changed port requires `down` before starting again. `down` stops only this preview project and removes its exact port mapping; it does not release the Agent VM lease.

## Direct Compose use inside a VM

When already working in the synced Agent VM workspace, the equivalent command is:

```bash
GRAFANA_PREVIEW_PORT=3101 \
GRAFANA_PREVIEW_URL=http://127.0.0.1:3101 \
docker compose -p cvm-grafana-preview \
  -f ops/observability/preview/compose.yml up -d --wait
```

Do not point this compose file at a production Prometheus or attach production secrets. This preview is for dashboard layout, query shape, responsive review, and local Grafana interaction only; it is not production evidence or a performance certificate.

## Classified trace fixture

Open `/d/cvm-proxy-cases?from=now-30m&to=now&var-service=codex-vibe-monitor&var-environment=synthetic-preview&var-instance=fixture&var-endpoint=responses&var-trace_id=11111111111111111111111111111111`. Five clearly synthetic response roots model normal, slow, high-wait, retry and error classifications; their child phases and a late persistence interval exercise the actual Tempo/Grafana query and waterfall surfaces. They do not measure application behavior or overhead. `fixture:8040` is a private tenant query bridge, with no published port; Grafana alone is exposed as anonymous Viewer on the current VM mapping. Application export and credential isolation are checked separately by the existing Actions acceptance fixture.
