'use client';

import { useState, useRef, useEffect, useCallback } from 'react';
import Link from 'next/link';
import Image from 'next/image';
import { usePathname } from 'next/navigation';
import {
  LayoutDashboard, ListOrdered, Cpu, Users, Star, Rocket,
  Workflow, LayoutTemplate, History, Database,
  Activity, ScrollText, Bell, Plug, Key, Settings, Cable, Store,
  ChevronLeft, ChevronRight, Zap, Server, Plus,
  Sliders, Sparkles, Boxes,
} from 'lucide-react';
import { useApp } from '@/contexts/AppContext';
import { NAV_SECTIONS, SYSTEM_METRICS } from '@/lib/constants';

const ICONS: Record<string, React.ElementType> = {
  LayoutDashboard, ListOrdered, Cpu, Users, Star, Rocket,
  Workflow, LayoutTemplate, History, Database,
  Activity, ScrollText, Bell, Plug, Key, Settings, Cable, Store, Zap, Server, Plus,
  Sliders, Sparkles, Boxes,
};

/* ── New button menu items ─────────────────────────── */
const NEW_MENU_ITEMS = [
  { id: 'workflow',   label: 'Build Workflow',     path: '/workflow/builder',                      icon: LayoutTemplate, color: '#2563EB' },
  { id: 'expert',     label: 'New PRISM',           path: '/experts/deploy',                        icon: Rocket,         color: '#D97706' },
  { id: 'dataset',    label: 'New Dataset',         path: '/data?action=new',                       icon: Database,       color: '#0EA5E9' },
  { id: 'mcp',        label: 'New MCP Server',      path: '/providers/connections?tab=mcp&action=new', icon: Server,      color: '#059669' },
];

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
            fontSize: 16, fontWeight: 800,
            letterSpacing: '-0.04em', lineHeight: 1,
            color: '#F04500',
          }}>
            kortecx
          </div>
          <div style={{
            fontSize: 7.5, fontWeight: 600, color: 'var(--text-4)',
            letterSpacing: '0.1em', textTransform: 'uppercase', marginTop: 2,
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
          padding: collapsed ? '6px 0' : '6px 10px 6px 12px',
          justifyContent: collapsed ? 'center' : 'flex-start',
          position: 'relative',
          margin: '1px 5px',
          width: 'calc(100% - 10px)',
          borderRadius: 5,
          cursor: 'pointer',
          textDecoration: 'none',
          fontSize: 13,
          fontWeight: isActive ? 650 : 520,
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

function NewButton({ collapsed }: { collapsed: boolean }) {
  const [open, setOpen] = useState(false);
  const btnRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const [menuPos, setMenuPos] = useState<{ top: number; left: number } | null>(null);

  useEffect(() => {
    if (open && btnRef.current) {
      const r = btnRef.current.getBoundingClientRect();
      setMenuPos({ top: r.top, left: r.right + 6 });
    }
  }, [open]);

  // Close on outside click
  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node) &&
          btnRef.current && !btnRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [open]);

  return (
    <>
      <button
        ref={btnRef}
        onClick={() => setOpen(p => !p)}
        style={{
          display: 'flex', alignItems: 'center', gap: 6,
          margin: '6px 6px 4px', padding: collapsed ? '5px 0' : '5px 10px',
          justifyContent: collapsed ? 'center' : 'flex-start',
          width: 'calc(100% - 16px)', borderRadius: 7,
          background: open ? 'var(--primary)' : 'var(--primary-dim)',
          border: `1.5px solid ${open ? 'var(--primary)' : 'rgba(240,69,0,0.2)'}`,
          color: open ? '#fff' : 'var(--primary)',
          fontSize: 12.5, fontWeight: 700, cursor: 'pointer',
          transition: 'all 0.15s',
        }}
      >
        <Plus size={15} strokeWidth={2.5} />
        {!collapsed && 'New'}
      </button>

      {/* Flyout menu — positioned to the right of the button */}
      {open && menuPos && (
        <div
          ref={menuRef}
          style={{
            position: 'fixed',
            top: menuPos.top,
            left: menuPos.left,
            zIndex: 9999,
            background: 'var(--bg-surface)',
            border: '1px solid var(--border-md)',
            borderRadius: 10,
            boxShadow: '0 12px 40px rgba(0,0,0,0.12), 0 2px 8px rgba(0,0,0,0.06)',
            padding: '6px',
            minWidth: 220,
          }}
        >
          <div style={{ padding: '4px 10px 6px', fontSize: 10, fontWeight: 700, color: 'var(--text-4)', textTransform: 'uppercase', letterSpacing: '0.08em' }}>
            Create New
          </div>
          {NEW_MENU_ITEMS.map(item => (
            <Link
              key={item.id}
              href={item.path}
              onClick={() => setOpen(false)}
              style={{
                display: 'flex', alignItems: 'center', gap: 10,
                padding: '8px 10px', borderRadius: 6,
                textDecoration: 'none', fontSize: 13, fontWeight: 500,
                color: 'var(--text-1)',
                transition: 'background 0.1s',
              }}
              onMouseEnter={e => { e.currentTarget.style.background = `${item.color}0a`; }}
              onMouseLeave={e => { e.currentTarget.style.background = 'transparent'; }}
            >
              <span style={{
                width: 28, height: 28, borderRadius: 6,
                background: `${item.color}10`,
                display: 'flex', alignItems: 'center', justifyContent: 'center',
                flexShrink: 0,
              }}>
                <item.icon size={14} color={item.color} />
              </span>
              <span>{item.label}</span>
            </Link>
          ))}
        </div>
      )}
    </>
  );
}

export default function LeftNavbar() {
  const pathname = usePathname();
  const { sidebarCollapsed, toggleSidebar, sidebarWidth, setSidebarWidth } = useApp();

  /* Track query string client-side only to avoid SSR hydration mismatch */
  const [clientSearch, setClientSearch] = useState('');
  useEffect(() => {
    requestAnimationFrame(() => setClientSearch(window.location.search));
    const onPop = () => setClientSearch(window.location.search);
    window.addEventListener('popstate', onPop);
    return () => window.removeEventListener('popstate', onPop);
  }, [pathname]);
  const w = sidebarCollapsed ? 48 : sidebarWidth;

  /* ── Drag-to-resize ─────────────────────────────── */
  const [dragging, setDragging] = useState(false);
  const [resizeHover, setResizeHover] = useState(false);

  const onResizeStart = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    setDragging(true);
    const onMove = (ev: MouseEvent) => {
      setSidebarWidth(ev.clientX);
    };
    const onUp = () => {
      setDragging(false);
      document.removeEventListener('mousemove', onMove);
      document.removeEventListener('mouseup', onUp);
      document.body.style.cursor = '';
      document.body.style.userSelect = '';
    };
    document.addEventListener('mousemove', onMove);
    document.addEventListener('mouseup', onUp);
    document.body.style.cursor = 'col-resize';
    document.body.style.userSelect = 'none';
  }, [setSidebarWidth]);
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
        transition: dragging ? 'none' : 'width 0.2s ease',
        zIndex: 40,
        overflowX: 'visible',
        overflowY: 'auto',
      }}
    >
      {/* Brand + Collapse Toggle */}
      <div style={{
        height: 48,
        display: 'flex',
        alignItems: 'center',
        justifyContent: sidebarCollapsed ? 'center' : 'space-between',
        padding: sidebarCollapsed ? '0' : '0 10px 0 16px',
        borderBottom: '1px solid var(--border)',
        flexShrink: 0,
        position: 'relative',
      }}>
        <KortecxLogo collapsed={sidebarCollapsed} />
        <button
          onClick={toggleSidebar}
          title={sidebarCollapsed ? 'Expand sidebar' : 'Collapse sidebar'}
          style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            width: 22,
            height: 22,
            borderRadius: 5,
            background: 'none',
            border: '1px solid var(--border)',
            cursor: 'pointer',
            color: 'var(--text-4)',
            flexShrink: 0,
            transition: 'color 0.15s, background 0.15s, border-color 0.15s',
            ...(sidebarCollapsed ? {
              position: 'absolute' as const,
              bottom: -11,
              left: '50%',
              transform: 'translateX(-50%)',
              background: 'var(--bg-surface)',
              zIndex: 2,
              width: 20,
              height: 20,
              borderRadius: 10,
            } : {}),
          }}
          onMouseEnter={e => {
            e.currentTarget.style.color = '#F04500';
            e.currentTarget.style.background = 'rgba(240,69,0,0.06)';
            e.currentTarget.style.borderColor = 'rgba(240,69,0,0.25)';
          }}
          onMouseLeave={e => {
            e.currentTarget.style.color = 'var(--text-4)';
            e.currentTarget.style.background = sidebarCollapsed ? 'var(--bg-surface)' : 'none';
            e.currentTarget.style.borderColor = 'var(--border)';
          }}
        >
          {sidebarCollapsed ? <ChevronRight size={12} /> : <ChevronLeft size={12} />}
        </button>
      </div>

      {/* New Button + Dashboard */}
      <NewButton collapsed={sidebarCollapsed} />
      <Link href="/dashboard" style={{
        display: 'flex', alignItems: 'center', gap: 8,
        padding: sidebarCollapsed ? '6px 0' : '6px 10px 6px 12px',
        justifyContent: sidebarCollapsed ? 'center' : 'flex-start',
        margin: '1px 5px 2px', width: 'calc(100% - 10px)', borderRadius: 5,
        textDecoration: 'none', fontSize: 13, fontWeight: pathname === '/dashboard' ? 650 : 520,
        color: pathname === '/dashboard' ? '#F04500' : 'var(--text-2)',
        background: pathname === '/dashboard' ? 'rgba(240,69,0,0.08)' : 'transparent',
        transition: 'background 0.12s, color 0.12s',
      }}>
        <LayoutDashboard size={15} strokeWidth={pathname === '/dashboard' ? 2.2 : 1.8} />
        {!sidebarCollapsed && <span>Dashboard</span>}
      </Link>

      {/* Nav Sections */}
      <div style={{ flex: 1, overflowY: 'auto', overflowX: 'visible', padding: '4px 0' }}>
        {NAV_SECTIONS.map(section => (
          <div key={section.id} style={{ marginBottom: 2 }}>
            {!sidebarCollapsed && (
              <div style={{
                padding: '5px 14px 1px',
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
                margin: '3px 8px',
                borderRadius: 1,
              }} />
            )}
            {section.items.map(item => {
              const [itemPath, itemQuery] = item.path.split('?');
              const matchesPath = itemQuery
                ? pathname === itemPath && clientSearch.includes(itemQuery)
                : pathname === itemPath || pathname.startsWith(itemPath + '/');
              const hasMoreSpecificMatch = !itemQuery && matchesPath && section.items.some(
                other => other.id !== item.id
                  && other.path.split('?')[0].startsWith(itemPath + '/')
                  && (pathname === other.path.split('?')[0] || pathname.startsWith(other.path.split('?')[0] + '/'))
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
        <div style={{ padding: '8px 12px', borderTop: '1px solid var(--border)' }}>
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

      {/* Resize handle */}
      {!sidebarCollapsed && (
        <div
          onMouseDown={onResizeStart}
          onMouseEnter={() => setResizeHover(true)}
          onMouseLeave={() => setResizeHover(false)}
          style={{
            position: 'absolute',
            top: 0,
            right: -2,
            bottom: 0,
            width: 4,
            cursor: 'col-resize',
            zIndex: 50,
            background: dragging || resizeHover ? 'rgba(240,69,0,0.3)' : 'transparent',
            transition: dragging ? 'none' : 'background 0.15s',
          }}
        />
      )}
    </aside>
  );
}
