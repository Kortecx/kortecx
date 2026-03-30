'use client';

import { useState, useEffect, useRef, useCallback, Suspense } from 'react';
import { useRouter, useSearchParams } from 'next/navigation';
import { motion, AnimatePresence, type Variants } from 'framer-motion';
import {
  Rocket, Check, Loader2, ChevronRight, ChevronLeft,
  Globe, Lock, Cpu, Sliders, FileText, User,
  ExternalLink, Zap, Star, X, Settings, Search, RefreshCw,
} from 'lucide-react';
import { PROVIDERS, ROLE_META, ROLE_DESCRIPTIONS } from '@/lib/constants';

/* ── Section color ─────────────────────────────────── */
const SECTION_COLOR = '#8b5cf6';

const slideVariants: Variants = {
  enter: (dir: number) => ({ x: dir > 0 ? 40 : -40, opacity: 0 }),
  center: { x: 0, opacity: 1, transition: { duration: 0.28, ease: 'easeOut' } },
  exit:  (dir: number) => ({ x: dir > 0 ? -40 : 40, opacity: 0, transition: { duration: 0.2 } }),
};

/* ── Data ──────────────────────────────────────────── */
type ModelSourceType = 'local' | 'provider';

const DEPLOY_ROLES = [
  'researcher','analyst','writer','coder','reviewer',
  'planner','legal','financial','medical','coordinator',
  'data-engineer','creative',
] as const;


const ALL_MODELS = PROVIDERS.flatMap(p =>
  p.models.map(m => ({
    id:           m.id,
    name:         m.name,
    providerId:   p.id,
    providerName: p.name,
    providerColor: p.color,
    context:      m.contextWindow,
    costIn:       m.costInputPer1k,
    costOut:      m.costOutputPer1k,
    capabilities: m.capabilities,
    maxOut:       m.maxOutputTokens,
    recommended:  m.id === 'claude-sonnet-4-6',
  })),
);

const PROVIDER_TABS = PROVIDERS.filter(p => p.models.length > 0);

const CAP_COLORS: Record<string, string> = {
  reasoning: '#8b5cf6', coding: '#06b6d4', analysis: '#0ea5e9',
  writing: '#10b981', fast: '#f59e0b', 'long-context': '#ec4899',
  vision: '#f97316', math: '#84cc16', multilingual: '#6366f1',
  'structured-output': '#14b8a6',
};

/* ── Shared style atoms ─────────────────────────────── */
const inputStyle: React.CSSProperties = {
  width: '100%', padding: '9px 12px',
  borderRadius: 8, border: '1px solid var(--border-md)',
  background: 'var(--bg-surface)', color: 'var(--text-1)',
  fontSize: 13, outline: 'none', boxSizing: 'border-box',
  transition: 'border-color 0.15s',
};

/* ── Sub-components ─────────────────────────────────── */
type FieldProps = {
  label: string;
  required?: boolean;
  hint?: React.ReactNode;
  children: React.ReactNode;
  error?: string;
};
function Field({ label, required, hint, children, error }: FieldProps) {
  return (
    <div>
      <label style={{ display: 'block', fontSize: 12, fontWeight: 600, color: 'var(--text-2)', marginBottom: 6, letterSpacing: '0.02em' }}>
        {label}
        {required && <span style={{ color: '#ef4444', marginLeft: 3 }}>*</span>}
      </label>
      {children}
      {hint && !error && <div style={{ fontSize: 11, color: 'var(--text-4)', marginTop: 4 }}>{hint}</div>}
      {error && <div style={{ fontSize: 11, color: '#ef4444', marginTop: 4 }}>{error}</div>}
    </div>
  );
}

/* ── Step indicator ─────────────────────────────────── */
const STEPS = [
  { n: 1, label: 'Configuration', icon: User },
  { n: 2, label: 'Model',         icon: Cpu },
  { n: 3, label: 'Prompt & Config', icon: Sliders },
  { n: 4, label: 'Review & Bundle', icon: Rocket },
];

function StepBar({ current }: { current: number }) {
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 0, marginBottom: 32 }}>
      {STEPS.map((s, i) => {
        const done    = current > s.n;
        const active  = current === s.n;
        const Icon    = s.icon;
        const dotColor = done ? SECTION_COLOR : active ? SECTION_COLOR : 'var(--border-md)';
        return (
          <div key={s.n} style={{ display: 'flex', alignItems: 'center', flex: i < STEPS.length - 1 ? 1 : 0 }}>
            <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 6, minWidth: 64 }}>
              <motion.div
                animate={{ scale: active ? 1.08 : 1 }}
                style={{
                  width: 34, height: 34, borderRadius: '50%',
                  background: done ? SECTION_COLOR : active ? `${SECTION_COLOR}18` : 'var(--bg-surface)',
                  border: `2px solid ${dotColor}`,
                  display: 'flex', alignItems: 'center', justifyContent: 'center',
                  transition: 'all 0.2s',
                }}
              >
                {done
                  ? <Check size={14} color="#fff" strokeWidth={2.5} />
                  : <Icon size={14} color={active ? SECTION_COLOR : 'var(--text-4)'} />}
              </motion.div>
              <div style={{ fontSize: 10, fontWeight: active ? 700 : 500, color: active ? SECTION_COLOR : 'var(--text-4)', textAlign: 'center', whiteSpace: 'nowrap' }}>
                {s.label}
              </div>
            </div>
            {i < STEPS.length - 1 && (
              <div style={{ flex: 1, height: 2, background: done ? SECTION_COLOR : 'var(--border-sm)', margin: '0 4px', marginBottom: 18, transition: 'background 0.3s' }} />
            )}
          </div>
        );
      })}
    </div>
  );
}

/* ── Role tooltip ──────────────────────────────────── */
function RoleTooltip({ label, anchorRect }: { label: string; anchorRect: DOMRect | null }) {
  if (!anchorRect) return null;
  return (
    <div style={{
      position: 'fixed',
      left: anchorRect.left + anchorRect.width / 2,
      top: anchorRect.bottom + 8,
      transform: 'translateX(-50%)',
      background: '#0d0d0d', color: '#fff',
      fontSize: 11, fontWeight: 500,
      padding: '6px 10px', borderRadius: 6,
      whiteSpace: 'nowrap', pointerEvents: 'none',
      zIndex: 9999, boxShadow: '0 4px 12px rgba(0,0,0,0.18)',
      maxWidth: 260,
    }}>
      <div style={{
        position: 'absolute', top: -4, left: '50%', transform: 'translateX(-50%)',
        width: 0, height: 0,
        borderLeft: '5px solid transparent', borderRight: '5px solid transparent',
        borderBottom: '5px solid #0d0d0d',
      }} />
      {label}
    </div>
  );
}

/* ── Role card ──────────────────────────────────────── */
function RoleCard({ id, selected, onSelect, customLabel }: { id: string; selected: boolean; onSelect: () => void; customLabel?: string }) {
  const meta = ROLE_META[id as keyof typeof ROLE_META];
  const description = ROLE_DESCRIPTIONS[id] ?? '';
  const [hovered, setHovered] = useState(false);
  const ref = useRef<HTMLButtonElement>(null);
  const [rect, setRect] = useState<DOMRect | null>(null);

  useEffect(() => {
    if (hovered && ref.current) {
      setRect(ref.current.getBoundingClientRect());
    } else {
      setRect(null);
    }
  }, [hovered]);

  return (
    <>
      <motion.button
        ref={ref}
        whileHover={{ y: -2 }} whileTap={{ scale: 0.97 }}
        onClick={onSelect}
        onMouseEnter={() => setHovered(true)}
        onMouseLeave={() => setHovered(false)}
        style={{
          display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center',
          gap: 6, padding: '14px 10px', borderRadius: 10, cursor: 'pointer',
          border: selected ? `1.5px solid ${SECTION_COLOR}` : '1px solid var(--border-sm)',
          background: selected ? `${SECTION_COLOR}12` : hovered ? `${SECTION_COLOR}08` : 'var(--bg-surface)',
          transition: 'all 0.18s ease', outline: 'none',
          position: 'relative',
        }}
      >
        <div style={{ fontSize: 22, lineHeight: 1, transition: 'transform 0.15s', transform: hovered ? 'scale(1.1)' : 'scale(1)' }}>{meta?.emoji ?? '⚙️'}</div>
        <div style={{ fontSize: 11, fontWeight: selected ? 700 : hovered ? 600 : 500, color: selected ? SECTION_COLOR : hovered ? SECTION_COLOR : 'var(--text-2)', textAlign: 'center', transition: 'color 0.15s, font-weight 0.15s' }}>
          {customLabel || meta?.label || id}
        </div>
        {selected && (
          <div style={{ position: 'absolute', top: 4, right: 4, width: 14, height: 14, borderRadius: '50%', background: SECTION_COLOR, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
            <Check size={8} color="#fff" strokeWidth={3} />
          </div>
        )}
      </motion.button>
      {hovered && description && <RoleTooltip label={description} anchorRect={rect} />}
    </>
  );
}

/* ── Custom Role Dialog ────────────────────────────── */
function CustomRoleDialog({ open, onSave, onClose, initialName, initialDescription }: {
  open: boolean;
  onSave: (name: string, description: string) => void;
  onClose: () => void;
  initialName: string;
  initialDescription: string;
}) {
  const [roleName, setRoleName] = useState(initialName);
  const [roleDesc, setRoleDesc] = useState(initialDescription);

  /* eslint-disable react-hooks/set-state-in-effect */
  useEffect(() => {
    if (open) { setRoleName(initialName); setRoleDesc(initialDescription); }
  }, [open, initialName, initialDescription]);
  /* eslint-enable react-hooks/set-state-in-effect */

  if (!open) return null;

  return (
    <div style={{
      position: 'fixed', inset: 0, background: 'rgba(7,7,26,0.85)',
      zIndex: 200, display: 'flex', alignItems: 'center', justifyContent: 'center',
    }}>
      <motion.div
        initial={{ opacity: 0, scale: 0.96 }}
        animate={{ opacity: 1, scale: 1 }}
        style={{
          width: 440, background: 'var(--bg-surface)', border: '1px solid var(--border-md)',
          borderRadius: 12, overflow: 'hidden', boxShadow: '0 24px 80px rgba(0,0,0,0.2)',
        }}
      >
        <div style={{ padding: '20px 24px', borderBottom: '1px solid var(--border)', display: 'flex', alignItems: 'center', gap: 10 }}>
          <div style={{ width: 36, height: 36, borderRadius: 8, background: `${SECTION_COLOR}15`, border: `1px solid ${SECTION_COLOR}30`, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
            <Settings size={18} color={SECTION_COLOR} />
          </div>
          <div>
            <div style={{ fontSize: 15, fontWeight: 700, color: 'var(--text-1)' }}>Custom Role</div>
            <div style={{ fontSize: 12, color: 'var(--text-3)', marginTop: 2 }}>Define a custom role for this Agent</div>
          </div>
        </div>
        <div style={{ padding: '20px 24px', display: 'flex', flexDirection: 'column', gap: 16 }}>
          <div>
            <label style={{ display: 'block', fontSize: 12, fontWeight: 600, color: 'var(--text-2)', marginBottom: 6 }}>
              Role Name <span style={{ color: '#ef4444' }}>*</span>
            </label>
            <input
              value={roleName}
              onChange={e => setRoleName(e.target.value)}
              placeholder="e.g. Prompt Engineer, DevOps, Recruiter…"
              style={{
                width: '100%', padding: '9px 12px', borderRadius: 8,
                border: '1px solid var(--border-md)', background: 'var(--bg-surface)',
                color: 'var(--text-1)', fontSize: 13, outline: 'none', boxSizing: 'border-box',
              }}
            />
          </div>
          <div>
            <label style={{ display: 'block', fontSize: 12, fontWeight: 600, color: 'var(--text-2)', marginBottom: 6 }}>
              Description
            </label>
            <textarea
              value={roleDesc}
              onChange={e => setRoleDesc(e.target.value)}
              placeholder="Describe what this role specialises in…"
              rows={3}
              style={{
                width: '100%', padding: '9px 12px', borderRadius: 8,
                border: '1px solid var(--border-md)', background: 'var(--bg-surface)',
                color: 'var(--text-1)', fontSize: 13, outline: 'none', boxSizing: 'border-box',
                resize: 'vertical', lineHeight: 1.55,
              }}
            />
          </div>
        </div>
        <div style={{ padding: '14px 24px', borderTop: '1px solid var(--border)', display: 'flex', justifyContent: 'flex-end', gap: 8 }}>
          <button onClick={onClose} style={{
            padding: '8px 16px', borderRadius: 7, fontSize: 12,
            border: '1px solid var(--border-md)', background: 'transparent',
            color: 'var(--text-3)', cursor: 'pointer',
          }}>Cancel</button>
          <button
            onClick={() => { if (roleName.trim()) onSave(roleName.trim(), roleDesc.trim()); }}
            disabled={!roleName.trim()}
            style={{
              padding: '8px 18px', borderRadius: 7, fontSize: 12, fontWeight: 700,
              border: `1.5px solid ${SECTION_COLOR}`, background: `${SECTION_COLOR}15`,
              color: SECTION_COLOR, cursor: roleName.trim() ? 'pointer' : 'not-allowed',
              opacity: roleName.trim() ? 1 : 0.5,
            }}
          >Save Role</button>
        </div>
      </motion.div>
    </div>
  );
}

/* ── Model card ─────────────────────────────────────── */
function ModelCard({ model, selected, onSelect }: { model: typeof ALL_MODELS[0]; selected: boolean; onSelect: () => void }) {
  return (
    <motion.button
      whileHover={{ y: -1 }} whileTap={{ scale: 0.98 }}
      onClick={onSelect}
      style={{
        textAlign: 'left', padding: '14px 16px', borderRadius: 10, cursor: 'pointer',
        border: selected ? `1.5px solid ${SECTION_COLOR}` : '1px solid var(--border-sm)',
        background: selected ? `${SECTION_COLOR}08` : 'var(--bg-surface)',
        transition: 'all 0.15s', outline: 'none', position: 'relative',
      }}
    >
      {model.recommended && (
        <div style={{
          position: 'absolute', top: -8, right: 10,
          display: 'flex', alignItems: 'center', gap: 3,
          padding: '2px 8px', borderRadius: 20, fontSize: 9,
          fontWeight: 700, background: SECTION_COLOR, color: '#fff', letterSpacing: '0.04em',
        }}>
          <Star size={8} fill="#fff" strokeWidth={0} /> RECOMMENDED
        </div>
      )}
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 8 }}>
        <div style={{ fontSize: 13, fontWeight: 700, color: 'var(--text-1)' }}>{model.name}</div>
        {selected && (
          <div style={{ width: 18, height: 18, borderRadius: '50%', background: SECTION_COLOR, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
            <Check size={10} color="#fff" strokeWidth={2.5} />
          </div>
        )}
      </div>
      <div style={{ display: 'flex', gap: 12, fontSize: 11, color: 'var(--text-4)', marginBottom: 8 }}>
        <span>ctx {model.context >= 1_000_000 ? `${(model.context / 1_000_000).toFixed(1)}M` : `${(model.context / 1000).toFixed(0)}k`}</span>
        <span>in ${model.costIn}/1k</span>
        <span>out ${model.costOut}/1k</span>
      </div>
      <div style={{ display: 'flex', flexWrap: 'wrap', gap: 4 }}>
        {model.capabilities.slice(0, 4).map(cap => (
          <span key={cap} style={{
            fontSize: 9, fontWeight: 600, padding: '2px 6px', borderRadius: 4,
            background: `${CAP_COLORS[cap] ?? '#6b7280'}18`,
            color: CAP_COLORS[cap] ?? 'var(--text-3)',
            textTransform: 'uppercase', letterSpacing: '0.04em',
          }}>{cap}</span>
        ))}
      </div>
    </motion.button>
  );
}

/* ── Searchable model dropdown ──────────────────────── */
function ModelSearchDropdown({
  models, value, onChange, engine, error, onRefresh,
}: {
  models: Array<{ name: string; size: number }>;
  value: string;
  onChange: (name: string) => void;
  engine: string;
  error?: boolean;
  onRefresh?: () => void;
}) {
  const hasModels = models.length > 0;
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState('');
  const [highlightIdx, setHighlightIdx] = useState(0);
  const wrapRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  const filtered = models.filter(m =>
    m.name.toLowerCase().includes(search.toLowerCase()),
  );

  // Close on outside click
  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (wrapRef.current && !wrapRef.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [open]);

  useEffect(() => { setHighlightIdx(0); }, [search]); // eslint-disable-line react-hooks/set-state-in-effect -- reset on search change

  useEffect(() => {
    if (!open || !listRef.current) return;
    const el = listRef.current.children[highlightIdx] as HTMLElement | undefined;
    el?.scrollIntoView({ block: 'nearest' });
  }, [highlightIdx, open]);

  const handleKey = (e: React.KeyboardEvent) => {
    if (!hasModels) return;
    if (e.key === 'ArrowDown') { e.preventDefault(); setHighlightIdx(i => Math.min(filtered.length - 1, i + 1)); }
    else if (e.key === 'ArrowUp') { e.preventDefault(); setHighlightIdx(i => Math.max(0, i - 1)); }
    else if (e.key === 'Enter' && filtered[highlightIdx]) { e.preventDefault(); onChange(filtered[highlightIdx].name); setOpen(false); setSearch(''); }
    else if (e.key === 'Escape') { setOpen(false); setSearch(''); }
  };

  const selectedModel = models.find(m => m.name === value);

  /* No models available — show editable text input with search icon */
  if (!hasModels) {
    return (
      <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
        <div style={{
          ...inputStyle,
          display: 'flex', alignItems: 'center', gap: 8,
          borderColor: error ? '#ef4444' : undefined,
        }}>
          <Search size={13} color="var(--text-4)" style={{ flexShrink: 0 }} />
          <input
            value={value}
            onChange={e => onChange(e.target.value)}
            placeholder={engine === 'Ollama' ? 'llama3.1:8b' : 'loaded-model'}
            style={{
              border: 'none', outline: 'none', background: 'transparent',
              color: 'var(--text-1)', fontSize: 13, flex: 1, padding: 0,
            }}
          />
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: 6, fontSize: 11, color: 'var(--text-4)' }}>
          <span>Start {engine} to see available models, or type a model name manually</span>
          {onRefresh && (
            <button
              onClick={onRefresh}
              title="Refresh models"
              style={{
                display: 'inline-flex', alignItems: 'center', justifyContent: 'center',
                width: 22, height: 22, borderRadius: 4, border: '1px solid var(--border-sm)',
                background: 'var(--bg-surface)', cursor: 'pointer', flexShrink: 0, padding: 0,
              }}
            >
              <RefreshCw size={11} color="var(--text-4)" />
            </button>
          )}
        </div>
      </div>
    );
  }

  return (
    <div ref={wrapRef} style={{ position: 'relative' }}>
      {/* Trigger / input */}
      <div
        onClick={() => { setOpen(true); setTimeout(() => inputRef.current?.focus(), 0); }}
        style={{
          ...inputStyle,
          display: 'flex', alignItems: 'center', gap: 8, cursor: 'pointer',
          borderColor: error ? '#ef4444' : open ? SECTION_COLOR : undefined,
        }}
      >
        <Search size={13} color="var(--text-4)" style={{ flexShrink: 0 }} />
        {open ? (
          <input
            ref={inputRef}
            value={search}
            onChange={e => setSearch(e.target.value)}
            onKeyDown={handleKey}
            placeholder="Search models…"
            style={{
              border: 'none', outline: 'none', background: 'transparent',
              color: 'var(--text-1)', fontSize: 13, flex: 1, padding: 0,
            }}
          />
        ) : (
          <span style={{ flex: 1, color: value ? 'var(--text-1)' : 'var(--text-4)' }}>
            {value || 'Select a model…'}
          </span>
        )}
        {selectedModel && !open && selectedModel.size > 0 && (
          <span style={{ fontSize: 10, color: 'var(--text-4)', flexShrink: 0 }}>
            {(selectedModel.size / 1e9).toFixed(1)} GB
          </span>
        )}
        <ChevronRight size={12} color="var(--text-4)" style={{ transform: open ? 'rotate(90deg)' : 'none', transition: 'transform 0.15s', flexShrink: 0 }} />
      </div>

      {/* Dropdown */}
      {open && (
        <div
          ref={listRef}
          style={{
            position: 'absolute', top: '100%', left: 0, right: 0, marginTop: 4,
            background: 'var(--bg-surface)', border: '1px solid var(--border-md)',
            borderRadius: 8, boxShadow: '0 8px 24px rgba(0,0,0,0.12)',
            maxHeight: 260, overflowY: 'auto', zIndex: 100,
          }}
        >
          {filtered.length === 0 ? (
            <div style={{ padding: '14px 12px', fontSize: 12, color: 'var(--text-4)', textAlign: 'center' }}>
              No models match &ldquo;{search}&rdquo;
            </div>
          ) : (
            filtered.map((m, i) => (
              <div
                key={m.name}
                onClick={() => { onChange(m.name); setOpen(false); setSearch(''); }}
                onMouseEnter={() => setHighlightIdx(i)}
                style={{
                  display: 'flex', alignItems: 'center', justifyContent: 'space-between',
                  padding: '8px 12px', cursor: 'pointer',
                  background: i === highlightIdx ? `${SECTION_COLOR}12` : value === m.name ? `${SECTION_COLOR}08` : 'transparent',
                  transition: 'background 0.1s',
                }}
              >
                <span style={{ fontSize: 13, fontWeight: value === m.name ? 600 : 400, color: 'var(--text-1)' }}>
                  {m.name}
                </span>
                <span style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                  {m.size > 0 && (
                    <span style={{ fontSize: 10, color: 'var(--text-4)', padding: '1px 5px', borderRadius: 3, background: 'var(--bg-elevated)' }}>
                      {(m.size / 1e9).toFixed(1)} GB
                    </span>
                  )}
                  {value === m.name && <Check size={12} color={SECTION_COLOR} strokeWidth={2.5} />}
                </span>
              </div>
            ))
          )}
        </div>
      )}
    </div>
  );
}

/* ── Summary row ────────────────────────────────────── */
function SummaryRow({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <div style={{ display: 'flex', alignItems: 'flex-start', gap: 12, padding: '10px 0', borderBottom: '1px solid var(--border-sm)' }}>
      <div style={{ minWidth: 130, fontSize: 12, color: 'var(--text-4)', fontWeight: 600, paddingTop: 1 }}>{label}</div>
      <div style={{ fontSize: 13, color: 'var(--text-1)', flex: 1 }}>{value}</div>
    </div>
  );
}

/* ══════════════════════════════════════════════════════
   MAIN PAGE
══════════════════════════════════════════════════════ */
export default function DeployExpertPage() {
  return (
    <Suspense fallback={<div style={{ padding: 28 }}><Loader2 size={24} style={{ animation: 'spin 1s linear infinite' }} /></div>}>
      <DeployExpertPageInner />
    </Suspense>
  );
}

function DeployExpertPageInner() {
  const router = useRouter();
  const searchParams = useSearchParams();
  const connectTo = searchParams.get('connectTo');

  /* Step */
  const [step, setStep] = useState(1);

  /* Step 1 — config */
  const [name, setName]               = useState('');
  const [description, setDescription] = useState('');
  const [role, setRole]               = useState('researcher');
  const [category, setCategory]       = useState('custom');
  const [complexityLevel, setComplexityLevel] = useState(3);
  const [capabilities, setCapabilities] = useState<string[]>([]);
  const [specializations, setSpecializations] = useState('');
  const [showAdvanced, setShowAdvanced] = useState(false);
  /* Custom role */
  const [customRoleName, setCustomRoleName] = useState('');
  const [customRoleDescription, setCustomRoleDescription] = useState('');
  const [showCustomRoleDialog, setShowCustomRoleDialog] = useState(false);
  /* Tag chips */
  const [tagsList, setTagsList] = useState<string[]>([]);
  const [tagInput, setTagInput] = useState('');

  /* Step 2 — model source */
  const [modelSourceType, setModelSourceType] = useState<ModelSourceType>('provider');
  const [activeProvider, setActiveProvider] = useState(PROVIDER_TABS[0]?.id ?? 'anthropic');
  const [modelId, setModelId]               = useState('claude-sonnet-4-6');
  /* Local model config */
  const [localEngine, setLocalEngine]       = useState<'ollama' | 'llamacpp'>('ollama');
  const [localModelName, setLocalModelName] = useState('llama3.1:8b');
  const [localBaseUrl, setLocalBaseUrl]     = useState('');
  const [localModels, setLocalModels] = useState<Array<{ name: string; size: number }>>([]);
  const [localModelsLoading, setLocalModelsLoading] = useState(false);

  const ENGINE_URL = process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000';

  const fetchLocalModels = useCallback(async () => {
    setLocalModelsLoading(true);
    try {
      try {
        // Try engine backend first
        const r = await fetch(`${ENGINE_URL}/api/orchestrator/models/${localEngine}`);
        if (r.ok) {
          const data = await r.json();
          if (data.models?.length > 0) {
            setLocalModels(data.models);
            return;
          }
        }
      } catch { /* engine unreachable, try direct */ }

      // Fallback: fetch directly from the inference server
      try {
        const directUrl = localEngine === 'ollama'
          ? (localBaseUrl || 'http://localhost:11434')
          : (localBaseUrl || 'http://localhost:8080');

        if (localEngine === 'ollama') {
          const r = await fetch(`${directUrl}/api/tags`);
          if (r.ok) {
            const data = await r.json();
            const models = (data.models || []).map((m: Record<string, unknown>) => ({
              name: m.name as string,
              size: (m.size as number) || 0,
            }));
            if (models.length > 0) { setLocalModels(models); return; }
          }
        } else {
          const r = await fetch(`${directUrl}/v1/models`);
          if (r.ok) {
            const data = await r.json();
            const models = (data.data || []).map((m: Record<string, unknown>) => ({
              name: (m.id as string) || 'unknown',
              size: 0,
            }));
            if (models.length > 0) { setLocalModels(models); return; }
          }
        }
      } catch { /* inference server also unreachable */ }

      setLocalModels([]);
    } finally {
      setLocalModelsLoading(false);
    }
  }, [localEngine, localBaseUrl]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (modelSourceType !== 'local') return;
    fetchLocalModels();
  }, [modelSourceType, fetchLocalModels]);

  /* Step 3 — prompt & config */
  const [systemPrompt, setSystemPrompt] = useState('');
  const [temperature, setTemperature]   = useState(0.5);
  const [maxTokens, setMaxTokens]       = useState(4096);
  const [isPublic, setIsPublic]         = useState(false);

  /* Deploy state */
  const [deploying, setDeploying]     = useState(false);
  const [deployProgress, setDeployProgress] = useState(0);
  const [deployed, setDeployed]       = useState(false);
  const [deployedId, setDeployedId]   = useState('');
  const [error, setError]             = useState('');
  const [fieldErrors, setFieldErrors] = useState<Record<string, string>>({});

  /* API key check state */
  const [showApiKeyPrompt, setShowApiKeyPrompt] = useState(false);
  const [apiKeyMissing, setApiKeyMissing] = useState('');
  const [providerKeys, setProviderKeys] = useState<Record<string, boolean>>({});
  const [keysLoaded, setKeysLoaded] = useState(false);

  /* Fetch provider API key status on mount */
  useEffect(() => {
    fetch('/api/providers')
      .then(r => r.ok ? r.json() : { providers: [] })
      .then(data => {
        const keys: Record<string, boolean> = {};
        for (const p of (data.providers || [])) {
          keys[p.id || p.slug] = p.apiKeySet === true || p.connected === true;
        }
        setProviderKeys(keys);
        setKeysLoaded(true);
      })
      .catch(() => setKeysLoaded(true));
  }, []);

  /* Cache expert state to localStorage for resume after API key setup */
  const CACHE_KEY = 'kortecx_agent_bundle_cache';

  const cacheState = () => {
    const state = { name, description, role, category, complexityLevel, capabilities, specializations, modelSourceType, activeProvider, modelId, localEngine, localModelName, localBaseUrl, systemPrompt, temperature, maxTokens, tagsList, isPublic, step, customRoleName, customRoleDescription };
    localStorage.setItem(CACHE_KEY, JSON.stringify(state));
  };

  /* Restore cached state on mount */
  useEffect(() => {
    try {
      const raw = localStorage.getItem(CACHE_KEY);
      if (raw) {
        const cached = JSON.parse(raw);
        if (cached.name) setName(cached.name);
        if (cached.description) setDescription(cached.description);
        if (cached.role) setRole(cached.role);
        if (cached.modelSourceType) setModelSourceType(cached.modelSourceType);
        if (cached.activeProvider) setActiveProvider(cached.activeProvider);
        if (cached.modelId) setModelId(cached.modelId);
        if (cached.localEngine) setLocalEngine(cached.localEngine);
        if (cached.localModelName) setLocalModelName(cached.localModelName);
        if (cached.localBaseUrl) setLocalBaseUrl(cached.localBaseUrl);
        if (cached.systemPrompt) setSystemPrompt(cached.systemPrompt);
        if (cached.temperature != null) setTemperature(cached.temperature);
        if (cached.maxTokens) setMaxTokens(cached.maxTokens);
        if (cached.tagsList) setTagsList(cached.tagsList);
        if (cached.isPublic != null) setIsPublic(cached.isPublic);
        if (cached.category) setCategory(cached.category);
        if (cached.complexityLevel != null) setComplexityLevel(cached.complexityLevel);
        if (cached.capabilities) setCapabilities(cached.capabilities);
        if (cached.specializations) setSpecializations(cached.specializations);
        if (cached.step) setStep(cached.step);
        if (cached.customRoleName) setCustomRoleName(cached.customRoleName);
        if (cached.customRoleDescription) setCustomRoleDescription(cached.customRoleDescription);
        localStorage.removeItem(CACHE_KEY);
      }
    } catch { /* ignore */ }
  }, []);

  /* Read template from marketplace clone */
  useEffect(() => {
    const raw = searchParams.get('template');
    if (!raw) return;
    try {
      const t = JSON.parse(decodeURIComponent(raw));
      if (t.name) setName(t.name);
      if (t.description) setDescription(t.description);
      if (t.role) setRole(t.role);
      if (t.systemPrompt) setSystemPrompt(t.systemPrompt);
      if (t.tags?.length) setTagsList(t.tags);
      if (t.capabilities?.length) setCapabilities(t.capabilities);
    } catch { /* ignore bad template */ }
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  /* Derived */
  const selectedModel   = ALL_MODELS.find(m => m.id === modelId);
  const providerModels  = ALL_MODELS.filter(m => m.providerId === activeProvider);

  /* ── Validation ── */
  function validateStep(n: number): boolean {
    const errs: Record<string, string> = {};
    if (n === 1) {
      if (!name.trim()) errs.name = 'Agent name is required';
      else if (name.trim().length < 2) errs.name = 'Name must be at least 2 characters';
    }
    if (n === 2) {
      if (modelSourceType === 'provider' && !modelId) errs.model = 'Please select a model';
      if (modelSourceType === 'local' && !localModelName.trim()) errs.model = 'Please enter a local model name';
    }
    setFieldErrors(errs);
    return Object.keys(errs).length === 0;
  }

  function nextStep() {
    if (!validateStep(step)) return;

    // On step 2 with provider source, check API key
    if (step === 2 && modelSourceType === 'provider') {
      const providerSlug = selectedModel?.providerId || activeProvider;
      const hasKey = providerKeys[providerSlug] || providerKeys[providerSlug.toLowerCase()];
      if (!hasKey && keysLoaded) {
        const providerName = PROVIDERS.find(p => p.id === providerSlug || p.slug === providerSlug)?.name || providerSlug;
        setApiKeyMissing(providerName);
        setShowApiKeyPrompt(true);
        return;
      }
    }

    setStep(s => Math.min(4, s + 1));
    setError('');
  }

  function prevStep() {
    setStep(s => Math.max(1, s - 1));
    setError('');
    setFieldErrors({});
  }

  /* ── Deploy ── */
  async function handleDeploy() {
    if (!name || !role) { setError('Name and role are required.'); return; }
    if (modelSourceType === 'provider' && !modelId) { setError('Please select a model.'); return; }
    if (modelSourceType === 'local' && !localModelName.trim()) { setError('Please enter a local model name.'); return; }
    setDeploying(true); setError(''); setDeployProgress(0);

    // Animate progress
    const interval = setInterval(() => {
      setDeployProgress(p => {
        if (p >= 85) { clearInterval(interval); return p; }
        return p + Math.random() * 18;
      });
    }, 400);

    try {
      const payload: Record<string, unknown> = {
        name: name.trim(),
        role,
        modelSource: modelSourceType,
        description: description.trim(),
        systemPrompt: systemPrompt.trim(),
        temperature,
        maxTokens,
        tags: tagsList,
        isPublic,
        category,
        complexityLevel,
        capabilities,
        customRoleDescription,
        specializations: specializations.split(',').map(s => s.trim()).filter(Boolean),
      };

      if (modelSourceType === 'local') {
        payload.modelId = localModelName;
        payload.providerId = localEngine;
        payload.localModelConfig = {
          engine: localEngine,
          model: localModelName,
          ...(localBaseUrl ? { baseUrl: localBaseUrl } : {}),
        };
      } else {
        payload.modelId = modelId;
        payload.providerId = selectedModel?.providerId ?? 'anthropic';
      }

      const res = await fetch('/api/experts', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(payload),
      });
      clearInterval(interval);
      if (!res.ok) throw new Error((await res.json()).error ?? 'Deploy failed');
      const data = await res.json();
      setDeployProgress(100);
      setTimeout(async () => {
        const newId = data.expert?.id ?? '';
        setDeployed(true);
        setDeployedId(newId);
        localStorage.removeItem(CACHE_KEY);

        // Auto-connect to parent node if created from graph
        if (connectTo && newId) {
          try {
            await fetch('/api/experts/graph', {
              method: 'POST',
              headers: { 'Content-Type': 'application/json' },
              body: JSON.stringify({ source: connectTo, target: newId }),
            });
          } catch { /* ignore */ }
        }

        // Auto-redirect to Agents after 2 seconds
        setTimeout(() => router.push('/experts'), 2000);
        // Log deployment
        fetch('/api/logs', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            level: 'info',
            message: `Expert "${name}" deployed (${modelSourceType === 'local' ? `${localEngine}/${localModelName}` : modelId})`,
            source: 'expert',
            metadata: { expertName: name, role, modelSource: modelSourceType, model: modelSourceType === 'local' ? localModelName : modelId },
          }),
        }).catch(() => {});
      }, 400);
    } catch (e: unknown) {
      clearInterval(interval);
      setError(e instanceof Error ? e.message : 'Deploy failed');
      setDeployProgress(0);
    } finally {
      setDeploying(false);
    }
  }

  const [direction, setDirection] = useState(1);

  function goNext() { setDirection(1); nextStep(); }
  function goBack() { setDirection(-1); prevStep(); }

  /* ══════════════════════════════════════════════════
     RENDER STEPS
  ══════════════════════════════════════════════════ */

  /* Step 1 */
  const Step1 = (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 22 }}>
      {connectTo && (
        <div style={{
          display: 'flex', alignItems: 'center', gap: 8,
          padding: '10px 14px', borderRadius: 8,
          background: 'rgba(217,119,6,0.08)', border: '1.5px solid rgba(217,119,6,0.25)',
          fontSize: 12, color: '#D97706',
        }}>
          <span style={{ fontSize: 14 }}>🔗</span>
          This Agent will be connected to <strong>{connectTo}</strong> in the graph
        </div>
      )}
      <Field label="Agent Name" required hint="A unique, memorable name for this Agent" error={fieldErrors.name}>
        <input
          value={name}
          onChange={e => { setName(e.target.value); setFieldErrors(f => ({ ...f, name: '' })); }}
          placeholder="e.g. ResearchPro, CodeForge, LegalAide…"
          style={{ ...inputStyle, borderColor: fieldErrors.name ? '#ef4444' : undefined }}
        />
      </Field>

      <Field label="Description" hint="A brief summary of what this Agent specialises in">
        <textarea
          value={description}
          onChange={e => setDescription(e.target.value)}
          placeholder="Deep web and document research with source verification…"
          rows={3}
          style={{ ...inputStyle, resize: 'vertical', lineHeight: 1.55 }}
        />
      </Field>

      <Field label="Role" required hint="Defines the Agent's primary function and persona">
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)', gap: 8, marginTop: 4 }}>
          {DEPLOY_ROLES.map(id => (
            <div key={id} style={{ position: 'relative' }}>
              <RoleCard id={id} selected={role === id && !customRoleName} onSelect={() => { setRole(id); setCustomRoleName(''); setCustomRoleDescription(''); }} />
            </div>
          ))}
          {/* Custom role button */}
          <div style={{ position: 'relative' }}>
            <RoleCard
              id="custom"
              selected={role === 'custom' && !!customRoleName}
              customLabel={customRoleName || 'Custom'}
              onSelect={() => setShowCustomRoleDialog(true)}
            />
          </div>
        </div>
      </Field>

      <CustomRoleDialog
        open={showCustomRoleDialog}
        initialName={customRoleName}
        initialDescription={customRoleDescription}
        onClose={() => setShowCustomRoleDialog(false)}
        onSave={(rName, rDesc) => {
          setCustomRoleName(rName);
          setCustomRoleDescription(rDesc);
          setRole('custom');
          setShowCustomRoleDialog(false);
        }}
      />

      <Field label="Category" required hint="Dimension for graph-based grouping of related Agents">
        <div style={{ display: 'flex', flexWrap: 'wrap', gap: 6, marginTop: 4 }}>
          {['research', 'engineering', 'creative', 'analysis', 'operations', 'domain-specific', 'custom'].map(cat => {
            const active = category === cat;
            return (
              <button
                key={cat}
                type="button"
                onClick={() => setCategory(cat)}
                style={{
                  padding: '5px 12px', borderRadius: 6, fontSize: 11, fontWeight: active ? 600 : 500,
                  border: active ? `1.5px solid ${SECTION_COLOR}` : '1px solid var(--border-sm)',
                  background: active ? `${SECTION_COLOR}14` : 'var(--bg-surface)',
                  color: active ? SECTION_COLOR : 'var(--text-3)',
                  cursor: 'pointer', transition: 'all 0.15s',
                }}
              >
                {cat.charAt(0).toUpperCase() + cat.slice(1)}
              </button>
            );
          })}
        </div>
      </Field>

      <Field label="Tags" hint="Press Enter or Space to add a tag">
        <input
          value={tagInput}
          onChange={e => setTagInput(e.target.value)}
          onKeyDown={e => {
            if (e.key === 'Enter' || e.key === ' ') {
              e.preventDefault();
              const val = tagInput.trim();
              if (val && !tagsList.includes(val)) setTagsList(prev => [...prev, val]);
              setTagInput('');
            }
            if (e.key === 'Backspace' && !tagInput && tagsList.length > 0) {
              setTagsList(prev => prev.slice(0, -1));
            }
          }}
          placeholder={tagsList.length === 0 ? 'e.g. research, NLP, summarisation, RAG…' : 'Add another tag…'}
          style={inputStyle}
        />
        {tagsList.length > 0 && (
          <div style={{ display: 'flex', flexWrap: 'wrap', gap: 6, marginTop: 8 }}>
            {tagsList.map(tag => (
              <span key={tag} style={{
                display: 'inline-flex', alignItems: 'center', gap: 4,
                fontSize: 11, fontWeight: 600, padding: '3px 8px', borderRadius: 5,
                background: `${SECTION_COLOR}12`, color: SECTION_COLOR,
              }}>
                {tag}
                <button
                  type="button"
                  onClick={() => setTagsList(prev => prev.filter(t => t !== tag))}
                  style={{
                    background: 'none', border: 'none', cursor: 'pointer',
                    color: SECTION_COLOR, padding: 0, display: 'flex', alignItems: 'center',
                    opacity: 0.7,
                  }}
                >
                  <X size={10} strokeWidth={2.5} />
                </button>
              </span>
            ))}
          </div>
        )}
      </Field>

      {/* ── Collapsible Advanced Metadata ── */}
      <button
        type="button"
        onClick={() => setShowAdvanced(!showAdvanced)}
        style={{
          display: 'flex', alignItems: 'center', gap: 6,
          background: 'none', border: 'none', cursor: 'pointer',
          fontSize: 12, fontWeight: 600, color: 'var(--text-3)',
          padding: '4px 0',
        }}
      >
        <ChevronRight size={14} style={{ transform: showAdvanced ? 'rotate(90deg)' : 'none', transition: 'transform 0.2s' }} />
        Advanced Metadata
      </button>

      <AnimatePresence>
        {showAdvanced && (
          <motion.div
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: 'auto', opacity: 1, transition: { duration: 0.25 } }}
            exit={{ height: 0, opacity: 0, transition: { duration: 0.2 } }}
            style={{ overflow: 'hidden', display: 'flex', flexDirection: 'column', gap: 18 }}
          >
            <Field label="Complexity Level" hint="1 (simple) to 5 (complex) — affects graph node size">
              <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
                <input
                  type="range" min={1} max={5} step={1}
                  value={complexityLevel}
                  onChange={e => setComplexityLevel(Number(e.target.value))}
                  style={{ flex: 1, accentColor: SECTION_COLOR }}
                />
                <span style={{ fontSize: 14, fontWeight: 700, color: SECTION_COLOR, minWidth: 20, textAlign: 'center' }}>
                  {complexityLevel}
                </span>
              </div>
            </Field>

            <Field label="Capabilities" hint="Select what this Agent excels at">
              <div style={{ display: 'flex', flexWrap: 'wrap', gap: 6, marginTop: 4 }}>
                {['web-search', 'code-gen', 'data-analysis', 'document-writing', 'translation', 'reasoning', 'structured-output'].map(cap => {
                  const active = capabilities.includes(cap);
                  return (
                    <button
                      key={cap}
                      type="button"
                      onClick={() => setCapabilities(prev => active ? prev.filter(c => c !== cap) : [...prev, cap])}
                      style={{
                        padding: '5px 12px', borderRadius: 6, fontSize: 11, fontWeight: 500,
                        border: active ? `1.5px solid ${SECTION_COLOR}` : '1px solid var(--border-sm)',
                        background: active ? `${SECTION_COLOR}14` : 'var(--bg-surface)',
                        color: active ? SECTION_COLOR : 'var(--text-3)',
                        cursor: 'pointer', transition: 'all 0.15s',
                      }}
                    >
                      {cap}
                    </button>
                  );
                })}
              </div>
            </Field>

          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );

  /* Step 2 */
  const Step2 = (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
      {/* Model source toggle */}
      <div style={{ display: 'flex', gap: 8 }}>
        {([
          { key: 'local' as const, label: 'Local Inference', desc: 'Ollama / llama.cpp', color: '#059669' },
          { key: 'provider' as const, label: 'Cloud Provider', desc: 'Claude / GPT / Gemini', color: '#2563EB' },
        ]).map(opt => (
          <motion.button
            key={opt.key}
            whileHover={{ y: -1 }}
            onClick={() => { setModelSourceType(opt.key); setFieldErrors(f => ({ ...f, model: '' })); }}
            style={{
              flex: 1, padding: '16px 18px', borderRadius: 10, cursor: 'pointer',
              border: modelSourceType === opt.key ? `2px solid ${opt.color}` : '1px solid var(--border-sm)',
              background: modelSourceType === opt.key ? `${opt.color}08` : 'var(--bg-surface)',
              textAlign: 'left', transition: 'all 0.15s', outline: 'none',
            }}
          >
            <div style={{ fontSize: 14, fontWeight: 700, color: modelSourceType === opt.key ? opt.color : 'var(--text-1)', marginBottom: 4 }}>
              {opt.label}
            </div>
            <div style={{ fontSize: 11, color: 'var(--text-4)' }}>{opt.desc}</div>
            {modelSourceType === opt.key && (
              <div style={{ position: 'absolute', top: 8, right: 8, width: 18, height: 18, borderRadius: '50%', background: opt.color, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                <Check size={10} color="#fff" strokeWidth={2.5} />
              </div>
            )}
          </motion.button>
        ))}
      </div>

      {fieldErrors.model && (
        <div style={{ fontSize: 12, color: '#ef4444' }}>{fieldErrors.model}</div>
      )}

      {/* LOCAL MODEL CONFIG */}
      {modelSourceType === 'local' && (
        <motion.div
          initial={{ opacity: 0, y: 6 }} animate={{ opacity: 1, y: 0 }}
          style={{ display: 'flex', flexDirection: 'column', gap: 14 }}
        >
          <Field label="Inference Engine" required hint="Select the local inference server">
            <div style={{ display: 'flex', gap: 8, marginTop: 4 }}>
              {([
                { id: 'ollama' as const, label: 'Ollama', desc: 'Easy model management with ollama pull' },
                { id: 'llamacpp' as const, label: 'llama.cpp', desc: 'High-performance GGUF inference' },
              ]).map(eng => (
                <motion.button
                  key={eng.id}
                  whileHover={{ y: -1 }}
                  onClick={() => setLocalEngine(eng.id)}
                  style={{
                    flex: 1, padding: '12px 14px', borderRadius: 8, cursor: 'pointer', textAlign: 'left',
                    border: localEngine === eng.id ? `1.5px solid #059669` : '1px solid var(--border-sm)',
                    background: localEngine === eng.id ? '#05966908' : 'var(--bg-surface)',
                    outline: 'none', transition: 'all 0.15s',
                  }}
                >
                  <div style={{ fontSize: 13, fontWeight: 600, color: localEngine === eng.id ? '#059669' : 'var(--text-1)' }}>{eng.label}</div>
                  <div style={{ fontSize: 10, color: 'var(--text-4)', marginTop: 2 }}>{eng.desc}</div>
                </motion.button>
              ))}
            </div>
          </Field>

          <Field label="Model" required hint={
            <span style={{ display: 'inline-flex', alignItems: 'center', gap: 6 }}>
              {localModels.length > 0
                ? `${localModels.length} models available on ${localEngine}`
                : `Enter model name or start ${localEngine} to see available models`}
              {!localModelsLoading && (
                <button
                  onClick={fetchLocalModels}
                  title="Refresh model list"
                  style={{
                    display: 'inline-flex', alignItems: 'center', justifyContent: 'center',
                    width: 18, height: 18, borderRadius: 4, border: '1px solid var(--border-sm)',
                    background: 'transparent', cursor: 'pointer', padding: 0,
                  }}
                >
                  <RefreshCw size={10} color="var(--text-4)" />
                </button>
              )}
            </span>
          }>
            {localModelsLoading ? (
              <div style={{ ...inputStyle, display: 'flex', alignItems: 'center', gap: 8, color: 'var(--text-3)' }}>
                <Loader2 size={13} style={{ animation: 'spin 1s linear infinite' }} />
                Loading models...
              </div>
            ) : (
              <ModelSearchDropdown
                models={localModels}
                value={localModelName}
                onChange={v => { setLocalModelName(v); setFieldErrors(f => ({ ...f, model: '' })); }}
                engine={localEngine === 'ollama' ? 'Ollama' : 'llama.cpp'}
                error={!!fieldErrors.model}
                onRefresh={fetchLocalModels}
              />
            )}
          </Field>

          <Field label="Server URL" hint="Leave blank for default. Ollama: http://localhost:11434, llama.cpp: http://localhost:8080">
            <input
              value={localBaseUrl}
              onChange={e => setLocalBaseUrl(e.target.value)}
              placeholder={localEngine === 'ollama' ? 'http://localhost:11434' : 'http://localhost:8080'}
              style={inputStyle}
            />
          </Field>

          {/* Info bar */}
          <div style={{
            display: 'flex', alignItems: 'center', gap: 12, padding: '10px 14px',
            borderRadius: 8, background: '#05966908', border: '1px solid #05966920',
            fontSize: 12, color: 'var(--text-2)',
          }}>
            <Cpu size={14} color="#059669" />
            <span><strong style={{ color: 'var(--text-1)' }}>{localModelName || 'No model'}</strong> via {localEngine === 'ollama' ? 'Ollama' : 'llama.cpp'}</span>
            <span style={{ color: 'var(--text-4)' }}>·</span>
            <span style={{ color: '#059669', fontWeight: 600 }}>Free — runs locally</span>
          </div>
        </motion.div>
      )}

      {/* PROVIDER MODEL CONFIG */}
      {modelSourceType === 'provider' && (
        <motion.div
          initial={{ opacity: 0, y: 6 }} animate={{ opacity: 1, y: 0 }}
          style={{ display: 'flex', flexDirection: 'column', gap: 14 }}
        >
          {/* Provider tabs */}
          <div style={{ display: 'flex', gap: 4, padding: '4px', background: 'var(--bg-canvas, var(--bg-surface))', borderRadius: 10, border: '1px solid var(--border-sm)' }}>
            {PROVIDER_TABS.map(p => (
              <button
                key={p.id}
                onClick={() => setActiveProvider(p.id)}
                style={{
                  flex: 1, padding: '7px 12px', borderRadius: 7, cursor: 'pointer',
                  border: 'none', fontSize: 12, fontWeight: activeProvider === p.id ? 700 : 500,
                  background: activeProvider === p.id ? 'var(--bg-surface)' : 'transparent',
                  color: activeProvider === p.id ? p.color : 'var(--text-3)',
                  boxShadow: activeProvider === p.id ? '0 1px 3px rgba(0,0,0,0.1)' : 'none',
                  transition: 'all 0.15s',
                }}
              >
                {p.name}
              </button>
            ))}
          </div>

          <AnimatePresence mode="wait">
            <motion.div
              key={activeProvider}
              initial={{ opacity: 0, y: 6 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0 }}
              transition={{ duration: 0.18 }}
              style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(220px, 1fr))', gap: 10 }}
            >
              {providerModels.map(m => (
                <ModelCard
                  key={m.id}
                  model={m}
                  selected={modelId === m.id}
                  onSelect={() => { setModelId(m.id); setFieldErrors(f => ({ ...f, model: '' })); }}
                />
              ))}
              {providerModels.length === 0 && (
                <div style={{ gridColumn: '1/-1', textAlign: 'center', padding: '40px 0', color: 'var(--text-4)', fontSize: 13 }}>
                  No models available for this provider
                </div>
              )}
            </motion.div>
          </AnimatePresence>

          {selectedModel && (
            <div style={{
              display: 'flex', alignItems: 'center', gap: 16, padding: '10px 14px',
              borderRadius: 8, background: `${SECTION_COLOR}08`, border: `1px solid ${SECTION_COLOR}20`,
              fontSize: 12, color: 'var(--text-2)',
            }}>
              <Zap size={13} color={SECTION_COLOR} />
              <span><strong style={{ color: 'var(--text-1)' }}>{selectedModel.name}</strong> selected</span>
              <span style={{ color: 'var(--text-4)' }}>·</span>
              <span>Context: {selectedModel.context >= 1_000_000 ? `${(selectedModel.context / 1_000_000).toFixed(1)}M` : `${(selectedModel.context / 1000).toFixed(0)}k`} tokens</span>
              <span style={{ color: 'var(--text-4)' }}>·</span>
              <span>In: ${selectedModel.costIn}/1k · Out: ${selectedModel.costOut}/1k</span>
            </div>
          )}
        </motion.div>
      )}
    </div>
  );

  /* Step 3 */
  const Step3 = (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 20 }}>
      <Field label="System Prompt" hint="Instructions that define this expert's behaviour, persona, and constraints">
        <textarea
          value={systemPrompt}
          onChange={e => setSystemPrompt(e.target.value)}
          placeholder={`You are an expert ${ROLE_META[role as keyof typeof ROLE_META]?.label ?? role} with deep knowledge in your domain.\n\nYour primary responsibilities:\n- Analyse and process inputs with precision\n- Provide structured, well-reasoned outputs\n- Cite sources and explain your reasoning`}
          rows={8}
          style={{ ...inputStyle, resize: 'vertical', lineHeight: 1.6, fontFamily: 'inherit' }}
        />
      </Field>

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 16 }}>
        <Field label={`Temperature  —  ${temperature.toFixed(2)}`} hint="Higher = more creative, lower = more deterministic">
          <div style={{ padding: '8px 0' }}>
            <input
              type="range" min={0} max={1} step={0.05}
              value={temperature}
              onChange={e => setTemperature(Number(e.target.value))}
              style={{ width: '100%', accentColor: SECTION_COLOR, cursor: 'pointer' }}
            />
            <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 10, color: 'var(--text-4)', marginTop: 4 }}>
              <span>Precise (0.0)</span>
              <span style={{ color: SECTION_COLOR, fontWeight: 700 }}>{temperature.toFixed(2)}</span>
              <span>Creative (1.0)</span>
            </div>
          </div>
        </Field>

        <Field label="Max Output Tokens" hint="Maximum tokens this expert can generate per call">
          <select
            value={maxTokens}
            onChange={e => setMaxTokens(Number(e.target.value))}
            style={{
              ...inputStyle, cursor: 'pointer',
              appearance: 'none' as const,
              backgroundImage: `url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='12' height='12' viewBox='0 0 24 24' fill='none' stroke='%236b7280' stroke-width='2'%3E%3Cpolyline points='6 9 12 15 18 9'%3E%3C/polyline%3E%3C/svg%3E")`,
              backgroundRepeat: 'no-repeat',
              backgroundPosition: 'right 10px center',
              paddingRight: 30,
            }}
          >
            {[512, 1024, 2048, 4096, 8192, 16384, 32768].map(v => (
              <option key={v} value={v}>{v.toLocaleString()} tokens</option>
            ))}
          </select>
        </Field>
      </div>

      <div style={{ display: 'flex', alignItems: 'center', gap: 12, padding: '14px 16px', borderRadius: 10, border: '1px solid var(--border-sm)', background: 'var(--bg-surface)' }}>
        <button
          onClick={() => setIsPublic(!isPublic)}
          style={{
            width: 42, height: 24, borderRadius: 12, flexShrink: 0,
            background: isPublic ? SECTION_COLOR : 'var(--border-md)',
            border: 'none', cursor: 'pointer', position: 'relative', transition: 'background 0.2s',
          }}
        >
          <motion.div
            animate={{ left: isPublic ? 20 : 3 }}
            transition={{ type: 'spring', stiffness: 500, damping: 30 }}
            style={{ position: 'absolute', top: 3, width: 18, height: 18, borderRadius: '50%', background: '#fff' }}
          />
        </button>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
          {isPublic ? <Globe size={14} color={SECTION_COLOR} /> : <Lock size={14} color="var(--text-4)" />}
          <div>
            <div style={{ fontSize: 13, fontWeight: 600, color: 'var(--text-1)' }}>
              {isPublic ? 'Public Agent' : 'Private Agent'}
            </div>
            <div style={{ fontSize: 11, color: 'var(--text-4)' }}>
              {isPublic ? 'Visible in the public Agent Catalog — anyone can use this Agent' : 'Only you and your team can access this Agent'}
            </div>
          </div>
        </div>
      </div>
    </div>
  );

  /* Step 4 — Review & Deploy */
  const roleMeta = ROLE_META[role as keyof typeof ROLE_META];

  const Step4 = deployed ? (
    /* ── Success state ── */
    <motion.div
      initial={{ opacity: 0, scale: 0.95 }} animate={{ opacity: 1, scale: 1 }}
      style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 16, padding: '40px 20px', textAlign: 'center' }}
    >
      <motion.div
        initial={{ scale: 0 }} animate={{ scale: 1 }}
        transition={{ type: 'spring', stiffness: 400, damping: 20, delay: 0.1 }}
        style={{
          width: 64, height: 64, borderRadius: '50%',
          background: '#10b98120', border: '2px solid #10b981',
          display: 'flex', alignItems: 'center', justifyContent: 'center',
        }}
      >
        <Check size={28} color="#10b981" strokeWidth={2.5} />
      </motion.div>
      <div>
        <h2 style={{ fontSize: 20, fontWeight: 700, color: 'var(--text-1)', marginBottom: 6 }}>Agent Bundled!</h2>
        <p style={{ fontSize: 13, color: 'var(--text-3)', maxWidth: 340 }}>
          <strong style={{ color: 'var(--text-1)' }}>{name}</strong> is now live and ready to accept tasks on the platform.
        </p>
      </div>
      <div style={{ display: 'flex', gap: 10 }}>
        <button
          onClick={() => router.push(`/experts/${deployedId || 'mine'}`)}
          style={{
            display: 'flex', alignItems: 'center', gap: 6,
            padding: '10px 20px', borderRadius: 8, background: SECTION_COLOR,
            border: 'none', color: '#fff', fontSize: 13, fontWeight: 600, cursor: 'pointer',
          }}
        >
          <ExternalLink size={13} /> View Agent
        </button>
        <button
          onClick={() => router.push('/experts')}
          style={{
            padding: '10px 18px', borderRadius: 8, border: '1px solid var(--border-md)',
            background: 'transparent', color: 'var(--text-2)', fontSize: 13, cursor: 'pointer',
          }}
        >
          All Agents
        </button>
      </div>
    </motion.div>
  ) : (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 20 }}>
      {/* Summary card */}
      <div style={{
        borderRadius: 12, border: '1px solid var(--border-sm)',
        background: 'var(--bg-surface)', overflow: 'hidden',
      }}>
        <div style={{
          padding: '12px 18px', background: `${SECTION_COLOR}08`,
          borderBottom: '1px solid var(--border-sm)',
          display: 'flex', alignItems: 'center', gap: 8,
        }}>
          <FileText size={14} color={SECTION_COLOR} />
          <span style={{ fontSize: 13, fontWeight: 700, color: 'var(--text-1)' }}>Bundle Summary</span>
        </div>
        <div style={{ padding: '4px 18px 8px' }}>
          <SummaryRow label="Agent Name" value={<strong>{name}</strong>} />
          <SummaryRow
            label="Role"
            value={
              <span style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                <span style={{ fontSize: 16 }}>{roleMeta?.emoji}</span>
                {roleMeta?.label}
              </span>
            }
          />
          <SummaryRow label="Description" value={description || <em style={{ color: 'var(--text-4)' }}>No description</em>} />
          <SummaryRow
            label="Model Source"
            value={
              <span style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                {modelSourceType === 'local' ? <Cpu size={12} color="#059669" /> : <Globe size={12} color="#2563EB" />}
                {modelSourceType === 'local' ? 'Local Inference' : 'Cloud Provider'}
              </span>
            }
          />
          <SummaryRow
            label="Model"
            value={
              modelSourceType === 'local' ? (
                <span>
                  {localModelName}
                  <span style={{ color: 'var(--text-4)', fontSize: 11, marginLeft: 6 }}>
                    via {localEngine === 'ollama' ? 'Ollama' : 'llama.cpp'}
                    {localBaseUrl ? ` (${localBaseUrl})` : ''}
                  </span>
                </span>
              ) : (
                <span>
                  {selectedModel?.name}
                  <span style={{ color: 'var(--text-4)', fontSize: 11, marginLeft: 6 }}>
                    via {selectedModel?.providerName}
                  </span>
                </span>
              )
            }
          />
          <SummaryRow label="Temperature" value={`${temperature.toFixed(2)} — ${temperature <= 0.3 ? 'Precise' : temperature >= 0.7 ? 'Creative' : 'Balanced'}`} />
          <SummaryRow label="Max Tokens" value={maxTokens.toLocaleString()} />
          <SummaryRow
            label="System Prompt"
            value={
              systemPrompt
                ? <span style={{ fontFamily: 'monospace', fontSize: 12, color: 'var(--text-2)' }}>{systemPrompt.slice(0, 120)}{systemPrompt.length > 120 ? '…' : ''}</span>
                : <em style={{ color: 'var(--text-4)' }}>Using default prompt</em>
            }
          />
          <SummaryRow
            label="Tags"
            value={
              tagsList.length > 0
                ? (
                  <div style={{ display: 'flex', flexWrap: 'wrap', gap: 4 }}>
                    {tagsList.map(t => (
                      <span key={t} style={{ fontSize: 11, padding: '1px 6px', borderRadius: 4, background: `${SECTION_COLOR}12`, color: SECTION_COLOR }}>{t}</span>
                    ))}
                  </div>
                )
                : <em style={{ color: 'var(--text-4)' }}>No tags</em>
            }
          />
          <SummaryRow
            label="Visibility"
            value={
              <span style={{ display: 'flex', alignItems: 'center', gap: 5 }}>
                {isPublic ? <Globe size={12} color={SECTION_COLOR} /> : <Lock size={12} color="var(--text-4)" />}
                {isPublic ? 'Public' : 'Private'}
              </span>
            }
          />
        </div>
      </div>

      {/* Progress bar during deploy */}
      {deploying && (
        <div style={{ borderRadius: 8, overflow: 'hidden', background: 'var(--border-sm)' }}>
          <motion.div
            animate={{ width: `${deployProgress}%` }}
            transition={{ ease: 'easeOut' }}
            style={{ height: 4, background: SECTION_COLOR }}
          />
          <div style={{ padding: '8px 12px', fontSize: 12, color: 'var(--text-3)', display: 'flex', alignItems: 'center', gap: 6 }}>
            <Loader2 size={12} color={SECTION_COLOR} style={{ animation: 'spin 1s linear infinite' }} />
            Bundling Agent… {Math.round(deployProgress)}%
          </div>
        </div>
      )}

      {/* Error */}
      {error && (
        <div style={{
          padding: '10px 14px', borderRadius: 8,
          background: '#ef444410', border: '1px solid #ef444425',
          color: '#ef4444', fontSize: 13,
        }}>
          {error}
        </div>
      )}
    </div>
  );

  const stepContent = [Step1, Step2, Step3, Step4][step - 1];

  /* ══════════════════════════════════════════════════
     PAGE RENDER
  ══════════════════════════════════════════════════ */
  return (
    <div style={{ padding: 28, maxWidth: 860, margin: '0 auto' }}>
      {/* API Key Missing Dialog */}
      {showApiKeyPrompt && (
        <div style={{
          position: 'fixed', inset: 0, background: 'rgba(7,7,26,0.85)',
          zIndex: 200, display: 'flex', alignItems: 'center', justifyContent: 'center',
        }}>
          <motion.div
            initial={{ opacity: 0, scale: 0.96 }}
            animate={{ opacity: 1, scale: 1 }}
            style={{
              width: 460, background: 'var(--bg-surface)', border: '1px solid var(--border-md)',
              borderRadius: 12, overflow: 'hidden', boxShadow: '0 24px 80px rgba(0,0,0,0.2)',
            }}
          >
            <div style={{ padding: '20px 24px', borderBottom: '1px solid var(--border)' }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                <div style={{ width: 36, height: 36, borderRadius: 8, background: 'rgba(245,158,11,0.1)', border: '1px solid rgba(245,158,11,0.2)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                  <Lock size={18} color="#f59e0b" />
                </div>
                <div>
                  <div style={{ fontSize: 15, fontWeight: 700, color: 'var(--text-1)' }}>API Key Required</div>
                  <div style={{ fontSize: 12, color: 'var(--text-3)', marginTop: 2 }}>
                    {apiKeyMissing} requires an API key to proceed
                  </div>
                </div>
              </div>
            </div>
            <div style={{ padding: '20px 24px' }}>
              <p style={{ fontSize: 13, color: 'var(--text-2)', lineHeight: 1.6, margin: '0 0 16px' }}>
                To use <strong style={{ color: 'var(--text-1)' }}>{apiKeyMissing}</strong> models, you need to configure an API key.
                Your current Agent configuration will be saved so you can return after adding the key.
              </p>
              <div style={{ padding: '10px 14px', borderRadius: 6, background: 'var(--bg)', border: '1px solid var(--border)', fontSize: 11, color: 'var(--text-3)', marginBottom: 16 }}>
                <strong style={{ color: 'var(--text-2)' }}>Where to get a key:</strong>
                <div style={{ marginTop: 4 }}>
                  Visit your provider&apos;s dashboard to generate an API key, then add it in
                  Kortecx Settings → Provider Keys.
                </div>
              </div>
            </div>
            <div style={{ padding: '14px 24px', borderTop: '1px solid var(--border)', display: 'flex', justifyContent: 'flex-end', gap: 8 }}>
              <button onClick={() => setShowApiKeyPrompt(false)} style={{
                padding: '8px 16px', borderRadius: 7, fontSize: 12,
                border: '1px solid var(--border-md)', background: 'transparent',
                color: 'var(--text-3)', cursor: 'pointer',
              }}>
                Cancel
              </button>
              <button onClick={() => {
                cacheState();
                setShowApiKeyPrompt(false);
                router.push('/providers/keys');
              }} style={{
                display: 'flex', alignItems: 'center', gap: 6,
                padding: '8px 18px', borderRadius: 7, fontSize: 12, fontWeight: 700,
                border: '1.5px solid #f59e0b', background: 'rgba(245,158,11,0.1)',
                color: '#f59e0b', cursor: 'pointer',
              }}>
                <ExternalLink size={12} /> Add API Key
              </button>
            </div>
          </motion.div>
        </div>
      )}

      {/* Page header */}
      <motion.div
        initial={{ opacity: 0, y: -8 }} animate={{ opacity: 1, y: 0 }}
        style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 28 }}
      >
        <div style={{
          width: 38, height: 38, borderRadius: 9,
          background: `${SECTION_COLOR}18`, border: `1.5px solid ${SECTION_COLOR}30`,
          display: 'flex', alignItems: 'center', justifyContent: 'center',
        }}>
          <Rocket size={18} color={SECTION_COLOR} strokeWidth={2} />
        </div>
        <div>
          <h1 style={{ fontSize: 19, fontWeight: 700, color: 'var(--text-1)', lineHeight: 1 }}>
            Bundle New Agent
          </h1>
          <p style={{ fontSize: 12, color: 'var(--text-3)', marginTop: 3 }}>
            Configure and bundle a new Agent — step {step} of 4
          </p>
        </div>
      </motion.div>

      {/* Step indicator */}
      <StepBar current={step} />

      {/* Step card */}
      <motion.div
        initial={{ opacity: 0, y: 10 }} animate={{ opacity: 1, y: 0 }}
        transition={{ delay: 0.05 }}
        style={{
          background: 'var(--bg-surface)', border: '1px solid var(--border-sm)',
          borderRadius: 14, padding: 28, minHeight: 360,
        }}
      >
        {/* Step heading */}
        <div style={{ marginBottom: 22 }}>
          <h2 style={{ fontSize: 15, fontWeight: 700, color: 'var(--text-1)' }}>
            Step {step} — {STEPS[step - 1].label}
          </h2>
          <div style={{ marginTop: 4, height: 2, width: 32, borderRadius: 1, background: SECTION_COLOR }} />
        </div>

        {/* Animated step content */}
        <AnimatePresence mode="wait" custom={direction}>
          <motion.div
            key={step}
            custom={direction}
            variants={slideVariants}
            initial="enter"
            animate="center"
            exit="exit"
          >
            {stepContent}
          </motion.div>
        </AnimatePresence>
      </motion.div>

      {/* Navigation buttons */}
      {!deployed && (
        <motion.div
          initial={{ opacity: 0 }} animate={{ opacity: 1 }} transition={{ delay: 0.15 }}
          style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginTop: 16 }}
        >
          <button
            onClick={step === 1 ? () => router.back() : goBack}
            style={{
              display: 'flex', alignItems: 'center', gap: 6,
              padding: '9px 18px', borderRadius: 8,
              border: '1px solid var(--border-md)', background: 'transparent',
              color: 'var(--text-3)', fontSize: 13, cursor: 'pointer',
            }}
          >
            <ChevronLeft size={14} />
            {step === 1 ? 'Cancel' : 'Back'}
          </button>

          <div style={{ display: 'flex', gap: 8 }}>
            {step < 4 ? (
              <button
                onClick={goNext}
                style={{
                  display: 'flex', alignItems: 'center', gap: 6,
                  padding: '9px 22px', borderRadius: 8,
                  background: SECTION_COLOR, border: 'none',
                  color: '#fff', fontSize: 13, fontWeight: 600, cursor: 'pointer',
                }}
              >
                Next <ChevronRight size={14} />
              </button>
            ) : (
              <button
                onClick={handleDeploy}
                disabled={deploying}
                style={{
                  display: 'flex', alignItems: 'center', gap: 8,
                  padding: '9px 24px', borderRadius: 8,
                  background: SECTION_COLOR, border: 'none',
                  color: '#fff', fontSize: 13, fontWeight: 600,
                  cursor: deploying ? 'wait' : 'pointer',
                  opacity: deploying ? 0.75 : 1,
                }}
              >
                {deploying
                  ? <><Loader2 size={14} style={{ animation: 'spin 1s linear infinite' }} /> Bundling…</>
                  : <><Rocket size={14} /> Bundle Agent</>}
              </button>
            )}
          </div>
        </motion.div>
      )}

      <style>{`
        @keyframes spin { from { transform: rotate(0deg); } to { transform: rotate(360deg); } }
      `}</style>
    </div>
  );
}
