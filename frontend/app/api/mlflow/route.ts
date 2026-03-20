import { NextRequest, NextResponse } from 'next/server';

const ENGINE_URL = process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000';

/* GET /api/mlflow — get MLflow status */
export async function GET() {
  try {
    const res = await fetch(`${ENGINE_URL}/api/mlflow/status`);
    if (!res.ok) return NextResponse.json({ enabled: false });
    return NextResponse.json(await res.json());
  } catch {
    return NextResponse.json({ enabled: false, error: 'Engine not reachable' });
  }
}

/* POST /api/mlflow — proxy log requests to engine */
export async function POST(req: NextRequest) {
  try {
    const body = await req.json();
    const { action, ...data } = body;

    const endpoint = action === 'dataset' ? '/api/mlflow/log/dataset'
      : action === 'chart' ? '/api/mlflow/log/chart'
      : action === 'model' ? '/api/mlflow/log/model'
      : action === 'asset' ? '/api/mlflow/log/asset'
      : null;

    if (!endpoint) {
      return NextResponse.json({ error: 'Invalid action' }, { status: 400 });
    }

    const res = await fetch(`${ENGINE_URL}${endpoint}`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(data),
    });
    return NextResponse.json(await res.json());
  } catch (err) {
    console.error('[mlflow POST]', err);
    return NextResponse.json({ error: 'MLflow log failed' }, { status: 500 });
  }
}
