export async function GET() {
  return Response.json({ message: "Hello from Rex API!", method: "GET" });
}

export async function POST() {
  return Response.json({ message: "Hello from Rex API!", method: "POST" });
}
