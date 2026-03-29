'use client';

import { ReactNode } from 'react';
import { usePathname } from 'next/navigation';
import { AppProvider, useApp } from '@/contexts/AppContext';
import LeftNavbar from './LeftNavbar';
import TopNavbar from './TopNavbar';

/* Routes that show no sidebar/topbar chrome */
const PUBLIC_PATHS: string[] = [];

function ShellContent({ children }: { children: ReactNode }) {
  const { sidebarCollapsed, sidebarWidth } = useApp();
  const pathname = usePathname();
  const left = sidebarCollapsed ? 48 : sidebarWidth;

  const isPublicPage = PUBLIC_PATHS.some(
    p => pathname === p || pathname.startsWith(p + '/')
  );

  if (isPublicPage) {
    return <>{children}</>;
  }

  return (
    <>
      <LeftNavbar />
      <TopNavbar />
      <main
        style={{
          marginLeft: left,
          paddingTop: 52,
          minHeight: '100vh',
          background: 'var(--bg-surface)',
          transition: 'margin-left 0.2s ease',
        }}
      >
        {children}
      </main>
    </>
  );
}

export default function AppShell({ children }: { children: ReactNode }) {
  return (
    <AppProvider>
      <ShellContent>{children}</ShellContent>
    </AppProvider>
  );
}
