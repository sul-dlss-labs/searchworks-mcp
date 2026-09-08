# Keycloak and Envoy boundary

`searchworks-mcp` intentionally does not authenticate users itself. Envoy should be the public endpoint and should forward authenticated MCP requests to the private Kubernetes Service.

The required behavior is API/MCP authentication, not only browser login:

1. A request without a token receives `401 Unauthorized`, not an HTML page or `302` redirect.
2. `WWW-Authenticate` identifies the OAuth protected-resource metadata URL.
3. Protected-resource and Keycloak authorization-server discovery documents are public.
4. Envoy validates issuer, audience, expiry, and the agreed role or scope on every request.
5. `/healthz` is only exposed inside the cluster. `/mcp` is the only public application route.
6. Envoy does not forward the caller's bearer token or cookies to SearchWorks.
7. Envoy preserves `MCP-Protocol-Version`, `Mcp-Method`, `Mcp-Name`, `Accept`, and `Content-Type`.

The Envoy OAuth2 HTTP filter commonly implements a redirect-and-cookie browser flow. That behavior alone is not sufficient for MCP clients. The final Envoy configuration may also require JWT authentication or external authorization filters. Exact Keycloak realm URLs, client IDs, audiences, scopes, TLS secrets, and ingress conventions are deployment-specific and are deliberately not guessed in this repository.
