'use client';

import { motion, HTMLMotionProps } from 'framer-motion';
import { ReactNode } from 'react';

interface GlowCardProps extends Omit<HTMLMotionProps<'div'>, 'children'> {
  children: ReactNode;
  glowColor?: string;
  hover?: boolean;
  className?: string;
  padding?: string;
}

export default function GlowCard({
  children,
  glowColor = 'rgba(0, 212, 255, 0.15)',
  hover = true,
  className = '',
  padding = 'p-5',
  ...props
}: GlowCardProps) {
  return (
    <motion.div
      whileHover={hover ? { scale: 1.005, y: -1 } : undefined}
      transition={{ duration: 0.2, ease: 'easeOut' }}
      className={`relative rounded-xl border border-[rgba(0,212,255,0.12)] bg-[#0a1628] ${padding} ${className}`}
      style={{
        transition: 'border-color 0.2s ease, box-shadow 0.2s ease',
      }}
      {...props}
    >
      {children}
    </motion.div>
  );
}
