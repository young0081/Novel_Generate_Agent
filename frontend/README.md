# Frontend

This Next.js UI is a local client for the Rust core. `npm run dev` and
`npm start` bind to `127.0.0.1`. Browser calls to `/api/rpc` must be loopback
and same-origin because the route can invoke file and process-backed tools;
local non-browser clients without an `Origin` header are also accepted.

Node.js 20.19 or newer is required.

Do not expose this server through a reverse proxy or public network without
adding authentication, authorization, CSRF protection, and TLS at the RPC
boundary.

The files, memory, and checkpoint panels include `批量管理`. Bulk deletion is
restricted to the currently listed files/records, runs sequentially through the
same RPC tools as single-item deletion, and reports progress plus partial
failures.
