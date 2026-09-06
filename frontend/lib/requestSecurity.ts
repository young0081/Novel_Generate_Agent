function parsedHost(host: string): string | null {
  try {
    return new URL(`http://${host}`).hostname.replace(/^\[|\]$/g, "").toLowerCase();
  } catch {
    return null;
  }
}

function isLoopbackHost(host: string): boolean {
  const hostname = parsedHost(host);
  return hostname === "localhost" || hostname === "127.0.0.1" || hostname === "::1";
}

/**
 * The RPC route controls local files and process-backed tools. It is therefore
 * local-only and rejects browser requests whose Origin does not match Host.
 */
export function isTrustedLocalRpcRequest(req: Request): boolean {
  const host = req.headers.get("host") ?? new URL(req.url).host;
  if (!isLoopbackHost(host)) return false;

  const origin = req.headers.get("origin");
  if (!origin) return true;
  try {
    const protocol = new URL(req.url).protocol;
    const expectedOrigin = new URL(`${protocol}//${host}`).origin;
    return new URL(origin).origin === expectedOrigin;
  } catch {
    return false;
  }
}
