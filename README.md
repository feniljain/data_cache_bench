# data_cache_bench

Benchmarks for the `foyer` hybrid on-disk cache.

## Running

```sh
cargo run --release
```

The app writes ~100 GB of random data into `./foyer-data/` and exposes foyer's internal metrics (prefixed `foyer_`) over HTTP for scraping.

## Metrics endpoint

- URL: `http://<host>:9091/metrics`
- Binds on `0.0.0.0:9091` so a containerized Prometheus can reach it.
- Content-Type: `text/plain; version=0.0.4` (standard Prometheus exposition format)

Quick check while the benchmark is running:

```sh
curl -s http://localhost:9091/metrics | grep ^foyer_ | head
```

## Configuring Prometheus to scrape it

Add a job to your Prometheus `prometheus.yml`:

```yaml
scrape_configs:
  - job_name: data_cache_bench
    scrape_interval: 5s
    static_configs:
      - targets: ['host.docker.internal:9091']  # Docker Desktop on macOS/Windows
        # targets: ['172.17.0.1:9091']          # Linux Docker default bridge
        # targets: ['localhost:9091']           # Prometheus running on host
```

Reload Prometheus (`docker kill -s HUP <container>` or restart it), then open
`http://localhost:9090/targets` — the `data_cache_bench` target should be
`UP`. Metrics are available in the query UI by typing `foyer_` and
autocompleting.

### Why `host.docker.internal`?

Your Prometheus runs in Docker on `:9090`; the bench binary runs on the host at
`:9091`. From inside the container, `localhost` is the container itself, so you
need a host-reachable address:

- **macOS / Windows (Docker Desktop)**: `host.docker.internal` is built in.
- **Linux**: either add `--add-host=host.docker.internal:host-gateway` to the
  prometheus container, or point the target at the docker bridge IP
  (`172.17.0.1` by default).
