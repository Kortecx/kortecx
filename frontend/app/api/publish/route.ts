import { NextResponse } from 'next/server';

export async function POST(request: Request) {
  const body = await request.json();
  const { content, platforms, scheduledAt } = body;

  return NextResponse.json({
    success: true,
    publishId: `pub-${Date.now()}`,
    content,
    platforms: platforms || [],
    status: scheduledAt ? 'scheduled' : 'published',
    publishedAt: scheduledAt || new Date().toISOString(),
  });
}

export async function GET() {
  return NextResponse.json({
    publications: [
      {
        id: 'pub-001',
        content: 'AI workflow automation update',
        platforms: ['twitter', 'linkedin'],
        status: 'published',
        publishedAt: '2026-03-15T10:00:00Z',
        engagement: { views: 1240, likes: 89, shares: 23 },
      },
      {
        id: 'pub-002',
        content: 'New expert deployment: LegalEagle',
        platforms: ['twitter'],
        status: 'published',
        publishedAt: '2026-03-14T14:00:00Z',
        engagement: { views: 820, likes: 45, shares: 12 },
      },
    ],
  });
}
