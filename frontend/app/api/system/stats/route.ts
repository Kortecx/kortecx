import { NextResponse } from 'next/server';

const ENGINE_URL = process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000';

/* GET /api/system/stats — proxy to engine for CPU/GPU/memory stats */
export async function GET() {
  try {
    const res = await fetch(`${ENGINE_URL}/api/orchestrator/system/stats`, {
      next: { revalidate: 0 },
    });
    if (!res.ok) return NextResponse.json({ cpu_percent: 0, memory_percent: 0, gpu: null });
    return NextResponse.json(await res.json());
  } catch {
    return NextResponse.json({ cpu_percent: 0, memory_percent: 0, gpu: null });
  }
}
