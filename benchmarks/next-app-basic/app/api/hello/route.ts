import { NextRequest, NextResponse } from 'next/server';

export async function GET(_request: NextRequest) {
  return NextResponse.json({ message: "Hello from Rex API!", method: "GET" });
}

export async function POST(_request: NextRequest) {
  return NextResponse.json({ message: "Hello from Rex API!", method: "POST" });
}
