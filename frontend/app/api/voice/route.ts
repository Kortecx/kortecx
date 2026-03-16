import { NextResponse } from 'next/server';

export async function POST(request: Request) {
  const body = await request.json();
  const { transcript, intent } = body;

  return NextResponse.json({
    success: true,
    command: {
      id: `cmd-${Date.now()}`,
      transcript: transcript || '',
      intent: intent || 'unknown',
      timestamp: new Date().toISOString(),
      status: 'processed',
    },
    result: {
      action: 'generate_content',
      content: `Generated response for: ${transcript}`,
    },
  });
}

export async function GET() {
  return NextResponse.json({
    supported_intents: [
      'generate_content',
      'run_workflow',
      'query_experts',
      'check_status',
      'create_task',
    ],
    voice_enabled: true,
  });
}
