export function jsonResponse(payload: unknown, init?: number | ResponseInit) {
  const responseInit = typeof init === 'number' ? { status: init } : init
  return new Response(JSON.stringify(payload), {
    status: responseInit?.status ?? 200,
    headers: {
      'Content-Type': 'application/json',
      ...(responseInit?.headers ?? {}),
    },
  })
}
