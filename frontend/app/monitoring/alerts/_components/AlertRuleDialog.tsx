'use client';

import { useState } from 'react';
import { X, Loader2, Bell, Webhook, Server, Link2 } from 'lucide-react';
import { motion } from 'framer-motion';
import { buttonHover } from '@/lib/motion';

type TriggerType = 'workflow_failure' | 'expert_error' | 'high_latency' | 'cost_threshold' | 'error_rate' | 'custom';
type Severity = 'info' | 'warning' | 'error' | 'critical';
type ChannelType = 'mcp_server' | 'integration' | 'webhook';

interface AlertRuleData {
  id?: string;
  name: string;
  description: string;
  triggerType: TriggerType;
  conditions: Record<string, unknown>;
  notificationConfig: { channel: ChannelType; targetId?: string; webhookUrl?: string; config?: Record<string, string> };
  severity: Severity;
  enabled: boolean;
  cooldownMinutes: number;
}

interface AlertRuleDialogProps {
  open: boolean;
  onClose: () => void;
  onSave: (rule: AlertRuleData) => Promise<void>;
  editRule?: AlertRuleData | null;
  mcpServers?: Array<{ id: string; name: string }>;
  integrations?: Array<{ id: string; name: string }>;
}

const TRIGGER_TYPES: { id: TriggerType; label: string }[] = [
  { id: 'workflow_failure', label: 'Workflow Failure' },
  { id: 'expert_error', label: 'Expert Error' },
  { id: 'high_latency', label: 'High Latency' },
  { id: 'cost_threshold', label: 'Cost Threshold' },
  { id: 'error_rate', label: 'Error Rate' },
  { id: 'custom', label: 'Custom' },
];

const SEVERITY_CONFIG: Record<Severity, { color: string; label: string }> = {
  info: { color: '#2563EB', label: 'Info' },
  warning: { color: '#D97706', label: 'Warning' },
  error: { color: '#DC2626', label: 'Error' },
  critical: { color: '#7C2D12', label: 'Critical' },
};

const CHANNEL_OPTIONS: { id: ChannelType; label: string; icon: typeof Server }[] = [
  { id: 'mcp_server', label: 'MCP Server', icon: Server },
  { id: 'integration', label: 'Integration', icon: Link2 },
  { id: 'webhook', label: 'Webhook', icon: Webhook },
];

export default function AlertRuleDialog({
  open, onClose, onSave, editRule, mcpServers = [], integrations = [],
}: AlertRuleDialogProps) {
  const editCond = (editRule?.conditions ?? {}) as Record<string, unknown>;
  const editNc = editRule?.notificationConfig ?? { channel: 'webhook' as ChannelType };
  const [name, setName] = useState(editRule?.name ?? '');
  const [description, setDescription] = useState(editRule?.description ?? '');
  const [triggerType, setTriggerType] = useState<TriggerType>(editRule?.triggerType ?? 'workflow_failure');
  const [severity, setSeverity] = useState<Severity>(editRule?.severity ?? 'warning');
  const [enabled, setEnabled] = useState(editRule?.enabled !== false);
  const [cooldownMinutes, setCooldownMinutes] = useState(editRule?.cooldownMinutes ?? 15);
  const [channel, setChannel] = useState<ChannelType>(editNc.channel);
  const [targetId, setTargetId] = useState(editNc.targetId ?? '');
  const [webhookUrl, setWebhookUrl] = useState(editNc.webhookUrl ?? '');
  const [threshold, setThreshold] = useState(editCond.threshold != null ? String(editCond.threshold) : '');
  const [operator, setOperator] = useState<'gt' | 'lt' | 'eq'>((editCond.operator as 'gt' | 'lt' | 'eq') ?? 'gt');
  const [saving, setSaving] = useState(false);

  if (!open) return null;

  const handleSave = async () => {
    if (!name.trim()) return;
    setSaving(true);

    const conditions: Record<string, unknown> = {};
    if (['high_latency', 'cost_threshold', 'error_rate'].includes(triggerType)) {
      conditions.operator = operator;
      conditions.threshold = parseFloat(threshold) || 0;
    }

    const notificationConfig = {
      channel,
      ...(channel === 'webhook' ? { webhookUrl } : { targetId }),
    };

    await onSave({
      ...(editRule?.id ? { id: editRule.id } : {}),
      name,
      description,
      triggerType,
      conditions,
      notificationConfig,
      severity,
      enabled,
      cooldownMinutes,
    });

    setSaving(false);
  };

  const needsThreshold = ['high_latency', 'cost_threshold', 'error_rate'].includes(triggerType);

  return (
    <div
      style={{
        position: 'fixed', inset: 0, background: 'rgba(0,0,0,0.6)',
        backdropFilter: 'blur(4px)', zIndex: 200,
        display: 'flex', alignItems: 'flex-start', justifyContent: 'center', paddingTop: 80,
      }}
      onClick={(e) => { if (e.target === e.currentTarget && !saving) onClose(); }}
    >
      <div onClick={(e) => e.stopPropagation()} style={{
        width: 520, maxWidth: '92vw', background: 'var(--bg-surface)',
        border: '1px solid var(--border)', borderRadius: 12, overflow: 'hidden',
        boxShadow: '0 24px 64px rgba(0,0,0,0.3)',
      }}>
        {/* Header */}
        <div style={{ padding: '18px 22px', borderBottom: '1px solid var(--border)', display: 'flex', alignItems: 'center', gap: 10 }}>
          <div style={{
            width: 36, height: 36, borderRadius: 8,
            background: 'rgba(220,38,38,0.1)', border: '1px solid rgba(220,38,38,0.25)',
            display: 'flex', alignItems: 'center', justifyContent: 'center',
          }}>
            <Bell size={18} color="#DC2626" />
          </div>
          <div style={{ flex: 1 }}>
            <div style={{ fontSize: 15, fontWeight: 700, color: 'var(--text-1)' }}>
              {editRule ? 'Edit Alert Rule' : 'Create Alert Rule'}
            </div>
            <div style={{ fontSize: 12, color: 'var(--text-3)' }}>
              Configure trigger conditions and notification channels
            </div>
          </div>
          <button onClick={onClose} disabled={saving}
            style={{ background: 'none', border: 'none', cursor: 'pointer', color: 'var(--text-3)', display: 'flex', padding: 4 }}>
            <X size={16} />
          </button>
        </div>

        {/* Body */}
        <div style={{ padding: '20px 22px', display: 'flex', flexDirection: 'column', gap: 14, maxHeight: '60vh', overflow: 'auto' }}>
          {/* Name */}
          <div>
            <label style={{ fontSize: 11, fontWeight: 600, color: 'var(--text-3)', marginBottom: 6, display: 'block' }}>Name</label>
            <input className="input" style={{ width: '100%', fontSize: 12 }}
              placeholder="e.g., Workflow Failure Alert" value={name} onChange={(e) => setName(e.target.value)} />
          </div>

          {/* Description */}
          <div>
            <label style={{ fontSize: 11, fontWeight: 600, color: 'var(--text-3)', marginBottom: 6, display: 'block' }}>Description</label>
            <input className="input" style={{ width: '100%', fontSize: 12 }}
              placeholder="Optional description" value={description} onChange={(e) => setDescription(e.target.value)} />
          </div>

          {/* Trigger type */}
          <div>
            <label style={{ fontSize: 11, fontWeight: 600, color: 'var(--text-3)', marginBottom: 6, display: 'block' }}>Trigger</label>
            <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
              {TRIGGER_TYPES.map(t => (
                <button key={t.id} onClick={() => setTriggerType(t.id)} style={{
                  padding: '5px 12px', borderRadius: 6, fontSize: 11, fontWeight: triggerType === t.id ? 600 : 400,
                  border: `1px solid ${triggerType === t.id ? '#DC2626' : 'var(--border)'}`,
                  background: triggerType === t.id ? 'rgba(220,38,38,0.08)' : 'transparent',
                  color: triggerType === t.id ? '#DC2626' : 'var(--text-3)',
                  cursor: 'pointer',
                }}>{t.label}</button>
              ))}
            </div>
          </div>

          {/* Threshold conditions (for applicable trigger types) */}
          {needsThreshold && (
            <div>
              <label style={{ fontSize: 11, fontWeight: 600, color: 'var(--text-3)', marginBottom: 6, display: 'block' }}>Condition</label>
              <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
                <span style={{ fontSize: 12, color: 'var(--text-2)' }}>
                  {triggerType === 'high_latency' ? 'Latency (ms)' : triggerType === 'cost_threshold' ? 'Cost (USD)' : 'Error rate (%)'}
                </span>
                <select className="input" style={{ width: 60, fontSize: 12 }} value={operator} onChange={(e) => setOperator(e.target.value as 'gt' | 'lt' | 'eq')}>
                  <option value="gt">&gt;</option>
                  <option value="lt">&lt;</option>
                  <option value="eq">=</option>
                </select>
                <input className="input" type="number" style={{ width: 80, fontSize: 12 }}
                  placeholder="0" value={threshold} onChange={(e) => setThreshold(e.target.value)} />
              </div>
            </div>
          )}

          {/* Severity */}
          <div>
            <label style={{ fontSize: 11, fontWeight: 600, color: 'var(--text-3)', marginBottom: 6, display: 'block' }}>Severity</label>
            <div style={{ display: 'flex', gap: 4 }}>
              {(Object.entries(SEVERITY_CONFIG) as [Severity, { color: string; label: string }][]).map(([sev, cfg]) => (
                <button key={sev} onClick={() => setSeverity(sev)} style={{
                  padding: '4px 10px', borderRadius: 5, fontSize: 11, fontWeight: severity === sev ? 600 : 400,
                  border: `1px solid ${severity === sev ? cfg.color : 'var(--border)'}`,
                  background: severity === sev ? `${cfg.color}14` : 'transparent',
                  color: severity === sev ? cfg.color : 'var(--text-3)',
                  cursor: 'pointer',
                }}>{cfg.label}</button>
              ))}
            </div>
          </div>

          {/* Notification channel */}
          <div>
            <label style={{ fontSize: 11, fontWeight: 600, color: 'var(--text-3)', marginBottom: 6, display: 'block' }}>Notification Channel</label>
            <div style={{ display: 'flex', gap: 4, marginBottom: 8 }}>
              {CHANNEL_OPTIONS.map(ch => {
                const Icon = ch.icon;
                return (
                  <button key={ch.id} onClick={() => setChannel(ch.id)} style={{
                    padding: '4px 10px', borderRadius: 5, fontSize: 11, fontWeight: channel === ch.id ? 600 : 400,
                    border: `1px solid ${channel === ch.id ? '#2563EB' : 'var(--border)'}`,
                    background: channel === ch.id ? 'rgba(37,99,235,0.08)' : 'transparent',
                    color: channel === ch.id ? '#2563EB' : 'var(--text-3)',
                    cursor: 'pointer', display: 'flex', alignItems: 'center', gap: 4,
                  }}>
                    <Icon size={10} /> {ch.label}
                  </button>
                );
              })}
            </div>

            {channel === 'webhook' && (
              <input className="input" style={{ width: '100%', fontSize: 12 }}
                placeholder="https://hooks.slack.com/..." value={webhookUrl} onChange={(e) => setWebhookUrl(e.target.value)} />
            )}
            {channel === 'mcp_server' && (
              mcpServers.length > 0 ? (
                <select className="input" style={{ width: '100%', fontSize: 12 }} value={targetId} onChange={(e) => setTargetId(e.target.value)}>
                  <option value="">Select MCP Server...</option>
                  {mcpServers.map(s => <option key={s.id} value={s.id}>{s.name}</option>)}
                </select>
              ) : (
                <div style={{ fontSize: 11, color: 'var(--text-4)', padding: '6px 0' }}>No MCP servers available. Create one in Connections.</div>
              )
            )}
            {channel === 'integration' && (
              integrations.length > 0 ? (
                <select className="input" style={{ width: '100%', fontSize: 12 }} value={targetId} onChange={(e) => setTargetId(e.target.value)}>
                  <option value="">Select Integration...</option>
                  {integrations.map(i => <option key={i.id} value={i.id}>{i.name}</option>)}
                </select>
              ) : (
                <div style={{ fontSize: 11, color: 'var(--text-4)', padding: '6px 0' }}>No integrations connected. Set up in Connections.</div>
              )
            )}
          </div>

          {/* Cooldown & enabled */}
          <div style={{ display: 'flex', gap: 16, alignItems: 'center' }}>
            <div style={{ flex: 1 }}>
              <label style={{ fontSize: 11, fontWeight: 600, color: 'var(--text-3)', marginBottom: 6, display: 'block' }}>Cooldown (minutes)</label>
              <input className="input" type="number" min={1} style={{ width: 80, fontSize: 12 }}
                value={cooldownMinutes} onChange={(e) => setCooldownMinutes(parseInt(e.target.value) || 15)} />
            </div>
            <div>
              <label style={{ fontSize: 11, fontWeight: 600, color: 'var(--text-3)', marginBottom: 6, display: 'block' }}>Enabled</label>
              <button onClick={() => setEnabled(p => !p)} style={{
                width: 40, height: 22, borderRadius: 11, border: 'none', cursor: 'pointer',
                background: enabled ? '#059669' : 'var(--border)',
                position: 'relative', transition: 'background 0.2s',
              }}>
                <span style={{
                  position: 'absolute', top: 2, left: enabled ? 20 : 2,
                  width: 18, height: 18, borderRadius: '50%', background: '#fff',
                  transition: 'left 0.2s', boxShadow: '0 1px 3px rgba(0,0,0,0.2)',
                }} />
              </button>
            </div>
          </div>
        </div>

        {/* Footer */}
        <div style={{ padding: '14px 22px', borderTop: '1px solid var(--border)', display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
          <button className="btn btn-secondary btn-sm" onClick={onClose} disabled={saving}>Cancel</button>
          <motion.button {...buttonHover} className="btn btn-primary btn-sm" onClick={handleSave}
            disabled={saving || !name.trim()}
            style={{ display: 'flex', alignItems: 'center', gap: 6, opacity: saving || !name.trim() ? 0.5 : 1 }}
          >
            {saving ? <Loader2 size={12} style={{ animation: 'spin 1s linear infinite' }} /> : <Bell size={12} />}
            {saving ? 'Saving...' : editRule ? 'Update Rule' : 'Create Rule'}
          </motion.button>
        </div>
      </div>
    </div>
  );
}
