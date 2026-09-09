# searchworks-mcp

A stateless Rust MCP server that presents Stanford SearchWorks catalog and article metadata as four read-only tools. It calls the existing SearchWorks JSON endpoints and projects their large Solr/EDS responses into small, safe MCP responses.

## Tools

- `catalog_search_tool`
- `article_search_tool`
- `get_catalog_record`
- `get_article`

The server sends no credentials upstream, and forwards no caller identity; access control is Envoy's job, per [docs/authentication.md](docs/authentication.md).

## Run locally

Rust 1.90 or newer:

```sh
cargo run
```

Or use Docker without installing Rust:

```sh
docker build -t searchworks-mcp:dev .
docker run --rm -p 3000:3000 searchworks-mcp:dev
curl http://localhost:3000/healthz
```

The MCP endpoint is `http://localhost:3000/mcp`.

## Configuration

| Variable | Default | Purpose |
| --- | --- | --- |
| `BIND_ADDRESS` | `0.0.0.0:3000` | HTTP listen address |
| `SEARCHWORKS_BASE_URL` | `https://searchworks.stanford.edu` | Upstream SearchWorks origin |
| `REQUEST_TIMEOUT_SECONDS` | `15` | Whole upstream request timeout |
| `MAX_RESPONSE_BYTES` | `2000000` | Maximum accepted upstream body |
| `UPSTREAM_USER_AGENT` | `searchworks-mcp/<version>` | Identifies service traffic |
| `MCP_ALLOWED_HOSTS` | local/service hostnames | Comma-separated accepted HTTP Host values; add the external Envoy hostname |
| `RUST_LOG` | application and HTTP info logs | Tracing filter |

The HTTP client rejects redirects and non-JSON responses, caps response size, redacts search parameters from logs, and does not maintain or forward cookies.

## Kubernetes

The starter manifests in `deploy/base` create two application replicas, a ClusterIP Service, health probes, conservative resource settings, a read-only filesystem, and a non-root container.

```sh
kubectl kustomize deploy/base
kubectl apply -k deploy/base
```

Before applying them:

1. Replace the example container image with an immutable tag from CI, i.e.
   `ghcr.io/<owner>/searchworks-mcp:sha-<commit>` rather than `:latest`.
2. Decide whether Envoy runs as the ingress/gateway or as a sidecar.
3. Configure Keycloak and Envoy using the requirements in [docs/authentication.md](docs/authentication.md).
4. Arrange a SearchWorks service quota and bot-challenge bypass. All calls otherwise share the Kubernetes egress IP.
5. Prefer adding a curated/versioned article-detail JSON response to SearchWorks; the current adapter necessarily mirrors part of `EdsDocument` parsing.

Do not expose the Kubernetes Service directly. Envoy should be the public authentication boundary.

## Development checks

```sh
make check
```

Because `Cargo.lock` is committed and the Docker build uses `--locked`, dependency versions are reproducible.

## Continuous integration

[`.github/workflows/ci.yml`](.github/workflows/ci.yml) runs on pull requests and on pushes to
`main`. It checks formatting, runs Clippy with warnings denied, and runs the tests against the
same Rust version the release image is built with.

The container image is built on every run, so a broken `Dockerfile` fails a pull request. Only a
push to `main` publishes it to GitHub Container Registry, tagged both `sha-<commit>` and
`latest`. Deploy the `sha-` tag: it is immutable, whereas `latest` moves with every merge.

Publishing uses the workflow's built-in `GITHUB_TOKEN`, so no registry secret has to be
configured. The package is private until made public in the repository's package settings.
