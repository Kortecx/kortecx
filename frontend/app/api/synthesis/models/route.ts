import { NextResponse } from 'next/server';

const ENGINE_URL = process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000';

export async function GET() {
  try {
    const res = await fetch(`${ENGINE_URL}/api/synthesis/models`);
    if (!res.ok) return NextResponse.json({ ollama: [], llamacpp: [], huggingface: [] });
    return NextResponse.json(await res.json());
  } catch {
    return NextResponse.json({ ollama: [], llamacpp: [], huggingface: [] });
  }
}
