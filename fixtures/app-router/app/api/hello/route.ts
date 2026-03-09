export function GET(req: { url: string; nextUrl: { searchParams: Record<string, string> } }) {
  const name = req.nextUrl?.searchParams?.name || 'world';
  return {
    statusCode: 200,
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ message: `Hello, ${name}!` }),
  };
}

export function POST(req: { body: unknown }) {
  return {
    statusCode: 201,
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ received: true, data: req.body }),
  };
}
