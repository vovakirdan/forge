# Local observability adjunct

Core and Supervisor are host binaries. Their metrics-only listeners default to
`127.0.0.1:9878` and `127.0.0.1:9879`; `--observability-address` changes each
listener. Non-loopback addresses are refused. No command, Gateway or inference
route is exposed on these ports. The owner-only Core UDS also has the three
operational endpoints.

Optional rootless Linux development adjunct:

```sh
podman-compose -f infra/dev/compose.observability.yaml -p forge-dev-observability up -d
curl --fail http://127.0.0.1:9090/-/ready
curl --fail http://127.0.0.1:9878/metrics
```

Prometheus is pinned to the v3.14.0 multi-architecture image manifest. It scrapes
only the two fixed loopback targets and listens only on loopback. The trusted
service uses the host network; this is never permitted for employee Run
containers. The named volume is retained for 15 days or 2 GB, whichever limit is
reached first. There is no remote-write receiver, Grafana, Loki or trace backend.

Stopping preserves metrics history:

```sh
podman-compose -f infra/dev/compose.observability.yaml -p forge-dev-observability down
```

Prometheus/exporter failure does not block canonical commands. Core still
reports process metrics when PostgreSQL fails; `forge_core_postgres_up=0` means
canonical count gauges may be stale. `up` and `resets(forge_core_uptime_seconds)`
provide scrape availability and observed restart information; neither is Run
execution evidence. Local loopback access shares the host's trust boundary,
not an authenticated remote-control API.
