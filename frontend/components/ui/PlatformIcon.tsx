'use client';

import { PLATFORMS } from '@/lib/constants';

interface PlatformIconProps {
  platformId: string;
  size?: number;
  showLabel?: boolean;
}

const PLATFORM_SYMBOLS: Record<string, string> = {
  twitter: 'X',
  linkedin: 'in',
  reddit: 'r/',
  discord: 'DC',
  telegram: 'TG',
  whatsapp: 'WA',
};

export default function PlatformIcon({ platformId, size = 28, showLabel = false }: PlatformIconProps) {
  const platform = PLATFORMS.find((p) => p.id === platformId);
  if (!platform) return null;

  const symbol = PLATFORM_SYMBOLS[platformId] || platformId.slice(0, 2).toUpperCase();
  const fontSize = size * 0.38;

  return (
    <div className="flex items-center gap-2">
      <div
        className="flex items-center justify-center rounded-lg font-mono font-bold flex-shrink-0"
        style={{
          width: size,
          height: size,
          backgroundColor: platform.bgColor,
          border: `1px solid ${platform.color}40`,
          color: platform.color,
          fontSize,
          letterSpacing: '-0.02em',
        }}
      >
        {symbol}
      </div>
      {showLabel && (
        <span className="text-sm text-[#94a3b8]">{platform.name}</span>
      )}
    </div>
  );
}
