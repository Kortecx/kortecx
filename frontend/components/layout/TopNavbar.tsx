'use client';

import { useState, useEffect, useCallback, useRef } from 'react';
import { usePathname } from 'next/navigation';
import { Search, Bell, Settings, ChevronRight, Activity } from 'lucide-react';
import Link from 'next/link';
import { useApp } from '@/contexts/AppContext';
import { SYSTEM_METRICS, ALERTS } from '@/lib/constants';
import SearchCommandDialog from './SearchCommandDialog';

/* ── Tooltip rendered below anchor ── */
function TopNavTooltip({ label, anchorRect }: { label: string; anchorRect: DOMRect | null }) {
  if (!anchorRect) return null;
  return (
    <div style={{
      position: 'fixed',
      left: anchorRect.left + anchorRect.width / 2,
      top: anchorRect.bottom + 8,
      transform: 'translateX(-50%)',
      background: '#0d0d0d',
      color: '#fff',
      fontSize: 12,
      fontWeight: 500,
      padding: '5px 10px',
      borderRadius: 5,
      whiteSpace: 'nowrap',
      pointerEvents: 'none',
      zIndex: 9999,
      boxShadow: '0 4px 12px rgba(0,0,0,0.18)',
    }}>
      {/* Arrow */}
      <div style={{
        position: 'absolute',
        top: -4,
        left: '50%',
        transform: 'translateX(-50%)',
        width: 0, height: 0,
        borderLeft: '5px solid transparent',
        borderRight: '5px solid transparent',
        borderBottom: '5px solid #0d0d0d',
      }} />
      {label}
    </div>
  );
}

const PRETTY: Record<string, string> = {
  dashboard:    'Dashboard',
  experts:      'Experts',
  workflow:     'Workflow',
  intelligence: 'Intelligence',
  finetuning:   'Fine-tuning',
  inference:    'Inference',
  models:       'Models',
  monitoring:   'Monitoring',
  providers:    'Providers',
  settings:     'Settings',
  data:         'Data Synthesis',
};

function buildBreadcrumb(path: string): Array<{ label: string; href: string }> {
  const segments = path.split('/').filter(Boolean);
  if (segments.length === 0) return [{ label: 'Dashboard', href: '/dashboard' }];
  const crumbs: Array<{ label: string; href: string }> = [];
  let current = '';
  for (const seg of segments) {
    current += `/${seg}`;
    crumbs.push({
      label: PRETTY[seg] ?? seg.charAt(0).toUpperCase() + seg.slice(1).replace(/-/g, ' '),
      href: current,
    });
  }
  return crumbs;
}

export default function TopNavbar() {
  const pathname = usePathname();
  const { sidebarCollapsed, sidebarWidth } = useApp();
  const left = sidebarCollapsed ? 48 : sidebarWidth;
  const crumbs = buildBreadcrumb(pathname);
  const unackAlerts = ALERTS.filter(a => !a.acknowledgedAt).length;
  const [searchOpen, setSearchOpen] = useState(false);

  /* Tooltip state for top-nav icons */
  const [monHovered, setMonHovered] = useState(false);
  const monRef = useRef<HTMLButtonElement>(null);
  const [monRect, setMonRect] = useState<DOMRect | null>(null);

  const [alertHovered, setAlertHovered] = useState(false);
  const alertRef = useRef<HTMLButtonElement>(null);
  const [alertRect, setAlertRect] = useState<DOMRect | null>(null);

  const [settingsHovered, setSettingsHovered] = useState(false);
  const settingsRef = useRef<HTMLButtonElement>(null);
  const [settingsRect, setSettingsRect] = useState<DOMRect | null>(null);

  /* eslint-disable react-hooks/set-state-in-effect */
  useEffect(() => {
    setMonRect(monHovered && monRef.current ? monRef.current.getBoundingClientRect() : null);
  }, [monHovered]);
  useEffect(() => {
    setAlertRect(alertHovered && alertRef.current ? alertRef.current.getBoundingClientRect() : null);
  }, [alertHovered]);
  useEffect(() => {
    setSettingsRect(settingsHovered && settingsRef.current ? settingsRef.current.getBoundingClientRect() : null);
  }, [settingsHovered]);
  /* eslint-enable react-hooks/set-state-in-effect */

  // Global ⌘K / Ctrl+K shortcut
  const handleGlobalKey = useCallback((e: KeyboardEvent) => {
    if ((e.metaKey || e.ctrlKey) && e.key === 'k') {
      e.preventDefault();
      setSearchOpen(prev => !prev);
    }
  }, [setSearchOpen]);

  useEffect(() => {
    document.addEventListener('keydown', handleGlobalKey);
    return () => document.removeEventListener('keydown', handleGlobalKey);
  }, [handleGlobalKey]);

  return (
    <>
    <header style={{
      position: 'fixed',
      top: 0,
      left: left,
      right: 0,
      height: 48,
      background: 'var(--bg-surface)',
      borderBottom: '1px solid var(--border)',
      display: 'flex',
      alignItems: 'center',
      padding: '0 20px',
      gap: 14,
      zIndex: 30,
      transition: 'left 0.2s ease',
    }}>
      {/* Left: Breadcrumb */}
      <nav style={{ display: 'flex', alignItems: 'center', gap: 4, flex: 1, minWidth: 0 }}>
        {crumbs.map((crumb, i) => (
          <span key={crumb.href} style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
            {i > 0 && <ChevronRight size={12} color="var(--text-4)" />}
            <Link
              href={crumb.href}
              style={{
                fontSize: 13,
                color: i === crumbs.length - 1 ? 'var(--text-1)' : 'var(--text-3)',
                fontWeight: i === crumbs.length - 1 ? 500 : 400,
                textDecoration: 'none',
                whiteSpace: 'nowrap',
              }}
            >
              {crumb.label}
            </Link>
          </span>
        ))}
      </nav>

      {/* Center: Search */}
      <button
        onClick={() => setSearchOpen(true)}
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 8,
          padding: '5px 14px',
          background: 'var(--bg-elevated)',
          border: '1px solid var(--border)',
          borderRadius: 6,
          cursor: 'pointer',
          color: 'var(--text-3)',
          fontSize: 12,
          transition: 'border-color 0.15s, background 0.15s',
          width: 320,
          flexShrink: 0,
        }}
        onMouseEnter={e => {
          e.currentTarget.style.borderColor = 'var(--border-strong)';
          e.currentTarget.style.background = '#ffffff';
        }}
        onMouseLeave={e => {
          e.currentTarget.style.borderColor = 'var(--border)';
          e.currentTarget.style.background = 'var(--bg-elevated)';
        }}
      >
        <Search size={13} />
        <span>Search...</span>
        <span style={{ marginLeft: 'auto', fontSize: 10, color: 'var(--text-4)' }}>⌘K</span>
      </button>

      {/* Right: System pulse + actions */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 10, flex: 1, justifyContent: 'flex-end', minWidth: 0 }}>
        {/* Agents & uptime */}
        <div style={{
          display: 'flex',
          alignItems: 'center',
          gap: 6,
          padding: '4px 10px',
          background: 'var(--bg-elevated)',
          border: '1px solid var(--border)',
          borderRadius: 4,
          fontSize: 11,
          color: 'var(--text-3)',
          flexShrink: 0,
        }}>
          <span style={{
            width: 6, height: 6, borderRadius: '50%',
            background: '#10b981',
            display: 'inline-block', flexShrink: 0,
            boxShadow: '0 0 0 2px rgba(16,185,129,0.20)',
            animation: 'pulse-dot 2s ease-in-out infinite',
          }} />
          <span className="mono" style={{ color: '#10b981', fontWeight: 600 }}>
            {SYSTEM_METRICS.activeAgents}
          </span>
          <span>active</span>
          <span style={{ color: 'var(--border-strong)' }}>·</span>
          <span className="mono" style={{ fontWeight: 500, color: 'var(--text-2)' }}>
            {(SYSTEM_METRICS.successRate * 100).toFixed(1)}%
          </span>
          <span>uptime</span>
        </div>

      {/* Monitoring link */}
      <Link href="/monitoring">
        <button
          ref={monRef}
          style={{
            background: 'none',
            border: 'none',
            cursor: 'pointer',
            color: 'var(--text-3)',
            display: 'flex',
            alignItems: 'center',
            borderRadius: 4,
            padding: 6,
            transition: 'color 0.15s, background 0.15s',
          }}
          onMouseEnter={e => {
            setMonHovered(true);
            e.currentTarget.style.color = '#DC2626';
            e.currentTarget.style.background = 'rgba(220,38,38,0.08)';
          }}
          onMouseLeave={e => {
            setMonHovered(false);
            e.currentTarget.style.color = 'var(--text-3)';
            e.currentTarget.style.background = 'none';
          }}
        >
          <Activity size={16} />
        </button>
      </Link>
      {monHovered && <TopNavTooltip label="Monitoring" anchorRect={monRect} />}

      {/* Alerts bell — colored when there are unread */}
      <Link href="/monitoring/alerts" style={{ position: 'relative' }}>
        <button
          ref={alertRef}
          style={{
            background: unackAlerts > 0 ? 'rgba(240,69,0,0.08)' : 'none',
            border: 'none',
            cursor: 'pointer',
            color: unackAlerts > 0 ? '#F04500' : 'var(--text-3)',
            display: 'flex',
            alignItems: 'center',
            borderRadius: 4,
            padding: 6,
            transition: 'color 0.15s, background 0.15s',
          }}
          onMouseEnter={e => {
            setAlertHovered(true);
            e.currentTarget.style.color = '#F04500';
            e.currentTarget.style.background = 'rgba(240,69,0,0.08)';
          }}
          onMouseLeave={e => {
            setAlertHovered(false);
            e.currentTarget.style.color = unackAlerts > 0 ? '#F04500' : 'var(--text-3)';
            e.currentTarget.style.background = unackAlerts > 0 ? 'rgba(240,69,0,0.08)' : 'none';
          }}
        >
          <Bell size={16} />
          {unackAlerts > 0 && (
            <span style={{
              position: 'absolute',
              top: 4, right: 4,
              width: 7, height: 7,
              borderRadius: '50%',
              background: '#DC2626',
              border: '1.5px solid var(--bg-surface)',
            }} />
          )}
        </button>
      </Link>
      {alertHovered && <TopNavTooltip label="Alerts" anchorRect={alertRect} />}

      {/* Settings */}
      <Link href="/settings">
        <button
          ref={settingsRef}
          style={{
            background: 'none',
            border: 'none',
            cursor: 'pointer',
            color: 'var(--text-3)',
            display: 'flex',
            alignItems: 'center',
            borderRadius: 4,
            padding: 6,
            transition: 'color 0.15s, background 0.15s',
          }}
          onMouseEnter={e => {
            setSettingsHovered(true);
            e.currentTarget.style.color = '#6b7280';
            e.currentTarget.style.background = 'rgba(107,114,128,0.08)';
          }}
          onMouseLeave={e => {
            setSettingsHovered(false);
            e.currentTarget.style.color = 'var(--text-3)';
            e.currentTarget.style.background = 'none';
          }}
        >
          <Settings size={16} />
        </button>
      </Link>
      {settingsHovered && <TopNavTooltip label="Settings" anchorRect={settingsRect} />}
      </div>{/* end right-side wrapper */}

    </header>

    {/* Search command dialog */}
    <SearchCommandDialog open={searchOpen} onClose={() => setSearchOpen(false)} />
    </>
  );
}
