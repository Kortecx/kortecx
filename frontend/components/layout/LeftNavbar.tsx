'use client';

import { useState, useRef, useEffect } from 'react';
import Link from 'next/link';
import Image from 'next/image';
import { usePathname } from 'next/navigation';
import {
  LayoutDashboard, ListOrdered, Cpu, Users, Star, Rocket,
  Workflow, LayoutTemplate, History, Brain, Database, Sliders,
  Activity, ScrollText, Bell, Plug, Key, Settings, Cable,
  ChevronLeft, ChevronRight,
} from 'lucide-react';
import { useApp } from '@/contexts/AppContext';
import { NAV_SECTIONS, SYSTEM_METRICS } from '@/lib/constants';

const ICONS: Record<string, React.ElementType> = {
  LayoutDashboard, ListOrdered, Cpu, Users, Star, Rocket,
  Workflow, LayoutTemplate, History, Brain, Database, Sliders,
  Activity, ScrollText, Bell, Plug, Key, Settings, Cable,
};

function KortecxLogo({ collapsed }: { collapsed: boolean }) {
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 9 }}>
      <div style={{
        width: 28, height: 28,
        borderRadius: 6,
        background: '#fff8f5',
        border: '1px solid rgba(240,69,0,0.15)',
        display: 'flex', alignItems: 'center', justifyContent: 'center',
        flexShrink: 0,
        overflow: 'hidden',
      }}>
        <Image
          src="/kortecx.png"
          alt="Kortecx"
          width={22}
          height={22}
          style={{ objectFit: 'contain' }}
          priority
        />
      </div>
      {!collapsed && (
        <div>
          <div style={{
            fontSize: 14, fontWeight: 800,
            letterSpacing: '-0.05em', lineHeight: 1,
            color: '#F04500',
          }}>
            kortecx
          </div>
          <div style={{
            fontSize: 9, fontWeight: 600, color: 'var(--text-3)',
            letterSpacing: '0.08em', textTransform: 'uppercase', marginTop: 2,
          }}>
            Intelligence Platform
          </div>
        </div>
      )}
    </div>
  );
}

/* ── Tooltip rendered via portal to avoid overflow clipping ── */
function NavTooltip({ label, anchorRect }: { label: string; anchorRect: DOMRect | null }) {
  if (!anchorRect) return null;
  return (
    <div style={{
      position: 'fixed',
      left: anchorRect.right + 10,
      top: anchorRect.top + anchorRect.height / 2,
      transform: 'translateY(-50%)',
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
        left: -4,
        top: '50%',
        transform: 'translateY(-50%)',
        width: 0, height: 0,
        borderTop: '5px solid transparent',
        borderBottom: '5px solid transparent',
        borderRight: '5px solid #0d0d0d',
      }} />
      {label}
    </div>
  );
}

function NavItem({
  item,
  section,
  isActive,
  collapsed,
}: {
  item: { id: string; label: string; path: string; icon: string; badge?: number };
  section: { color: string };
  isActive: boolean;
  collapsed: boolean;
}) {
  const Icon = ICONS[item.icon];
  const [hovered, setHovered] = useState(false);
  const ref = useRef<HTMLAnchorElement>(null);
  const [rect, setRect] = useState<DOMRect | null>(null);

  useEffect(() => {
    if (hovered && collapsed && ref.current) {
      setRect(ref.current.getBoundingClientRect());
    } else {
      setRect(null);
    }
  }, [hovered, collapsed]);

  return (
    <>
      <Link
        ref={ref}
        href={item.path}
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 8,
          padding: collapsed ? '7px 0' : '6px 10px 6px 14px',
          justifyContent: collapsed ? 'center' : 'flex-start',
          position: 'relative',
          margin: '0 6px',
          width: 'calc(100% - 12px)',
          borderRadius: 5,
          cursor: 'pointer',
          textDecoration: 'none',
          fontSize: 13,
          fontWeight: isActive ? 500 : 400,
          color: isActive ? section.color : (hovered ? section.color : 'var(--text-2)'),
          background: isActive
            ? `${section.color}12`
            : (hovered ? `${section.color}0a` : 'transparent'),
          transition: 'background 0.12s, color 0.12s',
          whiteSpace: 'nowrap',
          overflow: 'hidden',
        }}
        onMouseEnter={() => setHovered(true)}
        onMouseLeave={() => setHovered(false)}
      >
        {Icon && (
          <span style={{
            color: isActive || hovered ? section.color : 'var(--text-3)',
            flexShrink: 0,
            display: 'flex',
            transition: 'color 0.12s',
          }}>
            <Icon size={15} strokeWidth={isActive ? 2.2 : 1.8} />
          </span>
        )}
        {!collapsed && (
          <span style={{ flex: 1 }}>{item.label}</span>
        )}
        {!collapsed && item.badge !== undefined && item.badge > 0 && (
          <span style={{
            background: section.color,
            color: '#fff',
            fontSize: 10,
            fontWeight: 700,
            padding: '1px 6px',
            borderRadius: 10,
            minWidth: 18,
            textAlign: 'center',
            lineHeight: '16px',
          }}>
            {item.badge}
          </span>
        )}
        {collapsed && item.badge !== undefined && item.badge > 0 && (
          <span style={{
            position: 'absolute',
            top: 5, right: 5,
            width: 6, height: 6,
            borderRadius: '50%',
            background: section.color,
          }} />
        )}
      </Link>
      {collapsed && hovered && (
        <NavTooltip label={item.label} anchorRect={rect} />
      )}
    </>
  );
}

export default function LeftNavbar() {
  const pathname = usePathname();
  const { sidebarCollapsed, toggleSidebar } = useApp();
  const w = sidebarCollapsed ? 56 : 240;
  const tokPct = Math.round(
    (SYSTEM_METRICS.tokensUsedToday / SYSTEM_METRICS.tokenBudgetDaily) * 100
  );

  return (
    <aside
      style={{
        width: w,
        minWidth: w,
        position: 'fixed',
        top: 0, left: 0, bottom: 0,
        background: 'var(--bg-surface)',
        borderRight: '1px solid var(--border)',
        display: 'flex',
        flexDirection: 'column',
        transition: 'width 0.2s ease',
        zIndex: 40,
        overflowX: 'visible',
        overflowY: 'hidden',
      }}
    >
      {/* Brand */}
      <div style={{
        height: 52,
        display: 'flex',
        alignItems: 'center',
        justifyContent: sidebarCollapsed ? 'center' : 'flex-start',
        padding: sidebarCollapsed ? '0' : '0 16px',
        borderBottom: '1px solid var(--border)',
        flexShrink: 0,
      }}>
        <KortecxLogo collapsed={sidebarCollapsed} />
      </div>

      {/* Nav Sections */}
      <div style={{ flex: 1, overflowY: 'auto', overflowX: 'visible', padding: '8px 0' }}>
        {NAV_SECTIONS.map(section => (
          <div key={section.id} style={{ marginBottom: 4 }}>
            {!sidebarCollapsed && (
              <div style={{
                padding: '6px 16px 2px',
                fontSize: 9,
                fontWeight: 700,
                letterSpacing: '0.12em',
                color: section.color,
                opacity: 0.7,
              }}>
                {section.label}
              </div>
            )}
            {sidebarCollapsed && (
              <div style={{
                height: 1,
                background: `${section.color}22`,
                margin: '4px 10px',
                borderRadius: 1,
              }} />
            )}
            {section.items.map(item => {
              const matchesPath = pathname === item.path || pathname.startsWith(item.path + '/');
              const hasMoreSpecificMatch = matchesPath && section.items.some(
                other => other.id !== item.id
                  && other.path.startsWith(item.path + '/')
                  && (pathname === other.path || pathname.startsWith(other.path + '/'))
              );
              const isActive = matchesPath && !hasMoreSpecificMatch;
              return (
                <NavItem
                  key={item.id}
                  item={item}
                  section={section}
                  isActive={isActive}
                  collapsed={sidebarCollapsed}
                />
              );
            })}
          </div>
        ))}
      </div>

      {/* Token Usage */}
      {!sidebarCollapsed && (
        <div style={{ padding: '10px 14px', borderTop: '1px solid var(--border)' }}>
          <div style={{
            display: 'flex',
            justifyContent: 'space-between',
            fontSize: 10,
            color: 'var(--text-3)',
            marginBottom: 5,
          }}>
            <span>Daily Token Usage</span>
            <span className="mono" style={{ color: tokPct > 80 ? 'var(--warning)' : 'var(--text-2)' }}>
              {tokPct}%
            </span>
          </div>
          <div style={{
            height: 3, background: 'var(--bg-elevated)',
            borderRadius: 2, overflow: 'hidden',
          }}>
            <div style={{
              height: '100%',
              width: `${tokPct}%`,
              background: tokPct > 80 ? 'var(--warning)' : 'var(--primary)',
              borderRadius: 2,
              transition: 'width 0.4s ease',
            }} />
          </div>
          <div style={{
            display: 'flex',
            justifyContent: 'space-between',
            fontSize: 10,
            color: 'var(--text-4)',
            marginTop: 4,
          }}>
            <span className="mono">{(SYSTEM_METRICS.tokensUsedToday / 1000).toFixed(0)}k used</span>
            <span className="mono">{(SYSTEM_METRICS.tokenBudgetDaily / 1000).toFixed(0)}k limit</span>
          </div>
        </div>
      )}

      {/* Collapse Toggle — bottom */}
      <div style={{
        display: 'flex',
        alignItems: 'center',
        justifyContent: sidebarCollapsed ? 'center' : 'flex-end',
        padding: sidebarCollapsed ? '8px 0' : '8px 12px',
        borderTop: '1px solid var(--border)',
        flexShrink: 0,
      }}>
        <button
          onClick={toggleSidebar}
          title={sidebarCollapsed ? 'Expand sidebar' : 'Collapse sidebar'}
          style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            width: 28,
            height: 28,
            borderRadius: 6,
            background: 'none',
            border: '1px solid var(--border)',
            cursor: 'pointer',
            color: 'var(--text-3)',
            transition: 'color 0.15s, background 0.15s, border-color 0.15s',
          }}
          onMouseEnter={e => {
            e.currentTarget.style.color = 'var(--primary)';
            e.currentTarget.style.background = 'var(--primary-dim)';
            e.currentTarget.style.borderColor = 'var(--primary)';
          }}
          onMouseLeave={e => {
            e.currentTarget.style.color = 'var(--text-3)';
            e.currentTarget.style.background = 'none';
            e.currentTarget.style.borderColor = 'var(--border)';
          }}
        >
          {sidebarCollapsed ? <ChevronRight size={14} /> : <ChevronLeft size={14} />}
        </button>
      </div>
    </aside>
  );
}
