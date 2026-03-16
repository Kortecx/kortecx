import { NextResponse } from 'next/server';

const MOCK_PLATFORMS = [
  { id: 'twitter', name: 'X (Twitter)', connected: false, color: '#1d9bf0' },
  { id: 'linkedin', name: 'LinkedIn', connected: false, color: '#0a66c2' },
  { id: 'reddit', name: 'Reddit', connected: false, color: '#ff4500' },
  { id: 'discord', name: 'Discord', connected: false, color: '#5865f2' },
  { id: 'telegram', name: 'Telegram', connected: false, color: '#26a5e4' },
  { id: 'whatsapp', name: 'WhatsApp', connected: false, color: '#25d366' },
  { id: 'youtube', name: 'YouTube', connected: false, color: '#ff0000' },
  { id: 'instagram', name: 'Instagram', connected: false, color: '#e1306c' },
];

export async function GET() {
  return NextResponse.json({ platforms: MOCK_PLATFORMS });
}

export async function POST(request: Request) {
  const body = await request.json();
  const { platformId, action } = body;
  return NextResponse.json({
    success: true,
    platformId,
    action,
    status: action === 'connect' ? 'connected' : 'disconnected',
  });
}
