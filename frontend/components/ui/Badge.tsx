'use client';

interface BadgeProps {
  label: string;
  color?: string;
  bgColor?: string;
  dot?: boolean;
  size?: 'sm' | 'md';
}

export default function Badge({
  label,
  color = '#00d4ff',
  bgColor,
  dot = false,
  size = 'sm',
}: BadgeProps) {
  const bg = bgColor || `${color}18`;
  const textSize = size === 'sm' ? 'text-[10px]' : 'text-xs';
  const padding = size === 'sm' ? 'px-2 py-0.5' : 'px-2.5 py-1';

  return (
    <span
      className={`inline-flex items-center gap-1 rounded-full font-mono font-semibold uppercase tracking-wider ${textSize} ${padding}`}
      style={{ color, backgroundColor: bg, border: `1px solid ${color}30` }}
    >
      {dot && (
        <span
          className="w-1.5 h-1.5 rounded-full"
          style={{ backgroundColor: color, boxShadow: `0 0 4px ${color}` }}
        />
      )}
      {label}
    </span>
  );
}
