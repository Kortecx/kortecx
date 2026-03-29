'use client';

import { useState, useEffect } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import {
  AlertTriangle, AlertCircle, Info, ShieldAlert,
  Check, Clock, Loader2, Bell, Plus, Trash2, Edit3,
  Webhook, Server, Link2,
} from 'lucide-react';
import { useAlerts, useAlertRules } from '@/lib/hooks/useApi';
import { fadeUp, fadeDown, fadeRight, stagger, filterTab, rowEntrance, emptyState, buttonHover } from '@/lib/motion';
import type { Alert, AlertSeverity } from '@/lib/types';
import AlertRuleDialog from './_components/AlertRuleDialog';

const SEVERITY_CONFIG: Record<AlertSeverity, { icon: React.ElementType; color: string; bg: string }> = {
  info:     { icon: Info,         color: '#2563EB', bg: 'rgba(37,99,235,0.06)' },
  warning:  { icon: AlertTriangle, color: '#D97706', bg: 'rgba(217,119,6,0.06)' },
  error:    { icon: AlertCircle,  color: '#DC2626', bg: 'rgba(220,38,38,0.06)' },
  critical: { icon: ShieldAlert,  color: '#7C3AED', bg: 'rgba(124,58,237,0.06)' },
};

const TRIGGER_LABELS: Record<string, string> = {
  workflow_failure: 'Workflow Failure',
  expert_error: 'Expert Error',
  high_latency: 'High Latency',
  cost_threshold: 'Cost Threshold',
  error_rate: 'Error Rate',
  custom: 'Custom',
};

const CHANNEL_ICONS: Record<string, React.ElementType> = {
  mcp_server: Server,
  integration: Link2,
  webhook: Webhook,
};

function elapsed(iso: string): string {
  const diff = Math.floor((Date.now() - new Date(iso).getTime()) / 1000);
  if (diff < 60) return `${diff}s ago`;
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  return `${Math.floor(diff / 86400)}d ago`;
}

export default function AlertsPage() {
  const [activeTab, setActiveTab] = useState<'alerts' | 'rules'>('alerts');
  const [filter, setFilter] = useState<AlertSeverity | 'all'>('all');

  const { alerts, isLoading } = useAlerts() as { alerts: Alert[]; isLoading: boolean; error: unknown; mutate: () => void };
  const { rules, isLoading: rulesLoading, mutate: mutateRules } = useAlertRules();

  // Rule dialog state
  const [showRuleDialog, setShowRuleDialog] = useState(false);
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const [editingRule, setEditingRule] = useState<any>(null);
  const [mcpServers, setMcpServers] = useState<Array<{ id: string; name: string }>>([]);
  const [connectedIntegrations, setConnectedIntegrations] = useState<Array<{ id: string; name: string }>>([]);

  // Fetch MCP servers and integrations for notification channel options
  useEffect(() => {
    fetch('/api/mcp').then(r => r.json()).then(d => {
      const servers = [...(d.persisted || []), ...(d.prebuilt || [])].map((s: Record<string, unknown>) => ({
        id: s.id as string, name: s.name as string,
      }));
      setMcpServers(servers);
    }).catch(() => {});

    fetch('/api/oauth/connections').then(r => r.json()).then(d => {
      const connections = (d.connections || []).map((c: Record<string, unknown>) => ({
        id: c.id as string, name: (c.platformUsername || c.platform || c.id) as string,
      }));
      setConnectedIntegrations(connections);
    }).catch(() => {});
  }, []);

  const filtered = alerts.filter(a => filter === 'all' || a.severity === filter);
  const unacknowledged = alerts.filter(a => !a.acknowledgedAt).length;

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const handleSaveRule = async (ruleData: any) => {
    const isEdit = !!ruleData.id;
    const method = isEdit ? 'PATCH' : 'POST';
    await fetch('/api/alerts/rules', {
      method,
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(ruleData),
    });
    mutateRules();
    setShowRuleDialog(false);
    setEditingRule(null);
  };

  const handleDeleteRule = async (id: string) => {
    await fetch(`/api/alerts/rules?id=${id}`, { method: 'DELETE' });
    mutateRules();
  };

  const handleToggleRule = async (id: string, enabled: boolean) => {
    await fetch('/api/alerts/rules', {
      method: 'PATCH',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ id, enabled }),
    });
    mutateRules();
  };

  return (
    <div style={{ padding: 24, maxWidth: 1000, margin: '0 auto' }}>
      {/* Header */}
      <motion.div
        variants={fadeDown}
        initial="hidden"
        animate="show"
        style={{
          display: 'flex', alignItems: 'flex-start', justifyContent: 'space-between',
          marginBottom: 24,
        }}
      >
        <div>
          <h1 style={{ fontSize: 20, fontWeight: 700, color: 'var(--text-1)', margin: 0 }}>
            Alerts
          </h1>
          <p style={{ fontSize: 13, color: 'var(--text-3)', margin: '4px 0 0' }}>
            {isLoading ? '...' : `${alerts.length} total · ${unacknowledged} unacknowledged · ${rules.length} rule${rules.length !== 1 ? 's' : ''}`}
          </p>
        </div>
        <div style={{ display: 'flex', gap: 8 }}>
          {activeTab === 'alerts' && unacknowledged > 0 && (
            <motion.div variants={fadeRight} initial="hidden" animate="show" transition={{ delay: 0.15 }}>
              <button className="btn btn-secondary btn-sm">
                <Check size={13} /> Acknowledge All
              </button>
            </motion.div>
          )}
          {activeTab === 'rules' && (
            <motion.button {...buttonHover} className="btn btn-primary btn-sm" onClick={() => { setEditingRule(null); setShowRuleDialog(true); }}>
              <Plus size={13} /> Create Rule
            </motion.button>
          )}
        </div>
      </motion.div>

      {/* Tab bar */}
      <div style={{ display: 'flex', gap: 0, marginBottom: 20, borderBottom: '1px solid var(--border)' }}>
        {([
          { id: 'alerts' as const, label: 'Alerts', count: alerts.length },
          { id: 'rules' as const, label: 'Rules', count: rules.length },
        ]).map(tab => (
          <button key={tab.id} onClick={() => setActiveTab(tab.id)} style={{
            padding: '10px 20px', fontSize: 13, fontWeight: activeTab === tab.id ? 700 : 500,
            color: activeTab === tab.id ? 'var(--text-1)' : 'var(--text-3)',
            background: 'none', border: 'none', cursor: 'pointer',
            borderBottom: activeTab === tab.id ? '2px solid var(--text-1)' : '2px solid transparent',
            marginBottom: -1,
          }}>
            {tab.label}
            <span style={{
              marginLeft: 6, fontSize: 10, fontWeight: 700,
              padding: '1px 6px', borderRadius: 10,
              background: activeTab === tab.id ? 'var(--primary-dim)' : 'var(--bg)',
              color: activeTab === tab.id ? 'var(--primary-text)' : 'var(--text-4)',
            }}>{tab.count}</span>
          </button>
        ))}
      </div>

      {/* ── Alerts Tab ── */}
      {activeTab === 'alerts' && (
        <>
          {/* Severity filters */}
          <motion.div variants={stagger(0.04)} initial="hidden" animate="show" style={{ display: 'flex', gap: 6, marginBottom: 20 }}>
            <motion.button
              variants={fadeUp}
              {...filterTab}
              onClick={() => setFilter('all')}
              style={{
                padding: '5px 14px', borderRadius: 20, fontSize: 12, fontWeight: 500,
                border: '1px solid', cursor: 'pointer',
                background: filter === 'all' ? 'var(--primary-dim)' : 'var(--bg-card)',
                borderColor: filter === 'all' ? 'var(--primary)' : 'var(--border)',
                color: filter === 'all' ? 'var(--primary-text)' : 'var(--text-2)',
              }}
            >
              All ({alerts.length})
            </motion.button>
            {(Object.keys(SEVERITY_CONFIG) as AlertSeverity[]).map(severity => {
              const cfg = SEVERITY_CONFIG[severity];
              const count = alerts.filter(a => a.severity === severity).length;
              const active = filter === severity;
              return (
                <motion.button
                  key={severity}
                  variants={fadeUp}
                  {...filterTab}
                  onClick={() => setFilter(active ? 'all' : severity)}
                  style={{
                    padding: '5px 14px', borderRadius: 20, fontSize: 12, fontWeight: 500,
                    border: '1px solid', cursor: 'pointer', textTransform: 'capitalize',
                    background: active ? cfg.bg : 'var(--bg-card)',
                    borderColor: active ? cfg.color : 'var(--border)',
                    color: active ? cfg.color : 'var(--text-2)',
                  }}
                >
                  {severity} ({count})
                </motion.button>
              );
            })}
          </motion.div>

          {/* Alert list */}
          {isLoading ? (
            <motion.div
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              style={{ textAlign: 'center', padding: 60, color: 'var(--text-3)', fontSize: 13, display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 8 }}
            >
              <Loader2 size={16} className="animate-spin" /> Loading alerts...
            </motion.div>
          ) : (
            <AnimatePresence mode="popLayout">
              <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
                {filtered.map((alert, index) => {
                  const cfg = SEVERITY_CONFIG[alert.severity];
                  const Icon = cfg.icon;
                  const isAcknowledged = !!alert.acknowledgedAt;
                  const isResolved = !!alert.resolvedAt;

                  return (
                    <motion.div
                      key={alert.id}
                      {...rowEntrance(index)}
                      exit={{ opacity: 0, x: -30, transition: { duration: 0.25 } }}
                      layout
                    >
                      <div
                        className="card"
                        style={{
                          padding: '14px 18px',
                          borderLeft: `3px solid ${cfg.color}`,
                          opacity: isResolved ? 0.6 : 1,
                        }}
                      >
                        <div style={{ display: 'flex', alignItems: 'flex-start', gap: 12 }}>
                          <div style={{
                            width: 32, height: 32, borderRadius: 6,
                            background: cfg.bg, display: 'flex',
                            alignItems: 'center', justifyContent: 'center', flexShrink: 0,
                            marginTop: 2,
                          }}>
                            <Icon size={16} color={cfg.color} />
                          </div>
                          <div style={{ flex: 1 }}>
                            <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 3 }}>
                              <span style={{ fontSize: 14, fontWeight: 600, color: 'var(--text-1)' }}>
                                {alert.title}
                              </span>
                              <span style={{
                                fontSize: 10, fontWeight: 600, textTransform: 'uppercase',
                                letterSpacing: '0.06em', color: cfg.color,
                              }}>
                                {alert.severity}
                              </span>
                            </div>
                            <p style={{ fontSize: 12, color: 'var(--text-2)', margin: '0 0 8px', lineHeight: 1.5 }}>
                              {alert.message}
                            </p>
                            <div style={{ display: 'flex', alignItems: 'center', gap: 10, fontSize: 11, color: 'var(--text-4)' }}>
                              <span style={{ display: 'flex', alignItems: 'center', gap: 3 }}>
                                <Clock size={10} /> {elapsed(alert.createdAt)}
                              </span>
                              {isAcknowledged && (
                                <>
                                  <span style={{ color: 'var(--text-4)' }}>·</span>
                                  <span style={{ color: 'var(--success)', display: 'flex', alignItems: 'center', gap: 3 }}>
                                    <Check size={10} /> Acknowledged
                                  </span>
                                </>
                              )}
                              {isResolved && (
                                <>
                                  <span style={{ color: 'var(--text-4)' }}>·</span>
                                  <span style={{ color: 'var(--success)' }}>Resolved</span>
                                </>
                              )}
                            </div>
                          </div>
                          {!isResolved && (
                            <div style={{ display: 'flex', gap: 6, flexShrink: 0 }}>
                              {!isAcknowledged && (
                                <button className="btn btn-secondary btn-sm">
                                  <Check size={12} /> Ack
                                </button>
                              )}
                              <button className="btn btn-ghost btn-sm">
                                Resolve
                              </button>
                            </div>
                          )}
                        </div>
                      </div>
                    </motion.div>
                  );
                })}
              </div>
            </AnimatePresence>
          )}

          {!isLoading && filtered.length === 0 && (
            <motion.div
              {...emptyState}
              style={{ textAlign: 'center', padding: 60, color: 'var(--text-3)', fontSize: 14 }}
            >
              {alerts.length === 0 ? 'No alerts. System is healthy.' : 'No alerts match your filter.'}
            </motion.div>
          )}
        </>
      )}

      {/* ── Rules Tab ── */}
      {activeTab === 'rules' && (
        <>
          {rulesLoading ? (
            <motion.div
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              style={{ textAlign: 'center', padding: 60, color: 'var(--text-3)', fontSize: 13, display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 8 }}
            >
              <Loader2 size={16} className="animate-spin" /> Loading rules...
            </motion.div>
          ) : rules.length === 0 ? (
            <motion.div
              {...emptyState}
              style={{ textAlign: 'center', padding: 60, color: 'var(--text-3)', fontSize: 14 }}
            >
              <Bell size={28} color="var(--text-4)" style={{ margin: '0 auto 10px' }} />
              <div style={{ fontSize: 14, fontWeight: 600, color: 'var(--text-2)', marginBottom: 4 }}>No alert rules configured</div>
              <div style={{ fontSize: 12, maxWidth: 400, margin: '0 auto', lineHeight: 1.5 }}>
                Create rules to automatically trigger notifications when specific conditions are met.
              </div>
              <motion.button {...buttonHover} className="btn btn-primary btn-sm" style={{ marginTop: 16 }}
                onClick={() => { setEditingRule(null); setShowRuleDialog(true); }}>
                <Plus size={12} /> Create First Rule
              </motion.button>
            </motion.div>
          ) : (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
              {rules.map((rule: Record<string, unknown>, index: number) => {
                const severityCfg = SEVERITY_CONFIG[(rule.severity as AlertSeverity) || 'warning'] || SEVERITY_CONFIG.warning;
                const nc = (rule.notificationConfig || {}) as Record<string, unknown>;
                const ChannelIcon = CHANNEL_ICONS[(nc.channel as string) || 'webhook'] || Webhook;
                const isEnabled = rule.enabled !== false;

                return (
                  <motion.div
                    key={rule.id as string}
                    {...rowEntrance(index)}
                  >
                    <div className="card" style={{
                      padding: '14px 18px',
                      borderLeft: `3px solid ${isEnabled ? severityCfg.color : 'var(--border)'}`,
                      opacity: isEnabled ? 1 : 0.6,
                    }}>
                      <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
                        <div style={{
                          width: 32, height: 32, borderRadius: 6,
                          background: isEnabled ? severityCfg.bg : 'var(--bg)',
                          display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0,
                        }}>
                          <Bell size={16} color={isEnabled ? severityCfg.color : 'var(--text-4)'} />
                        </div>
                        <div style={{ flex: 1 }}>
                          <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 3 }}>
                            <span style={{ fontSize: 14, fontWeight: 600, color: 'var(--text-1)' }}>
                              {rule.name as string}
                            </span>
                            <span style={{
                              fontSize: 9, fontWeight: 700, padding: '1px 6px', borderRadius: 3,
                              background: `${severityCfg.color}12`, color: severityCfg.color,
                              textTransform: 'uppercase',
                            }}>
                              {rule.severity as string}
                            </span>
                            <span style={{
                              fontSize: 9, fontWeight: 700, padding: '1px 6px', borderRadius: 3,
                              background: 'var(--bg)', color: 'var(--text-3)',
                              textTransform: 'uppercase',
                            }}>
                              {TRIGGER_LABELS[(rule.triggerType as string)] || rule.triggerType as string}
                            </span>
                          </div>
                          <div style={{ display: 'flex', alignItems: 'center', gap: 8, fontSize: 11, color: 'var(--text-4)' }}>
                            <span style={{ display: 'flex', alignItems: 'center', gap: 3 }}>
                              <ChannelIcon size={10} /> {(nc.channel as string) || 'webhook'}
                            </span>
                            <span>·</span>
                            <span>Cooldown: {rule.cooldownMinutes as number}m</span>
                            {(rule.lastTriggeredAt as string) && (
                              <>
                                <span>·</span>
                                <span>Last: {elapsed(rule.lastTriggeredAt as string)}</span>
                              </>
                            )}
                          </div>
                        </div>
                        <div style={{ display: 'flex', gap: 6, alignItems: 'center', flexShrink: 0 }}>
                          {/* Enabled toggle */}
                          <button onClick={() => handleToggleRule(rule.id as string, !isEnabled)} style={{
                            width: 36, height: 20, borderRadius: 10, border: 'none', cursor: 'pointer',
                            background: isEnabled ? '#059669' : 'var(--border)',
                            position: 'relative', transition: 'background 0.2s',
                          }}>
                            <span style={{
                              position: 'absolute', top: 2, left: isEnabled ? 18 : 2,
                              width: 16, height: 16, borderRadius: '50%', background: '#fff',
                              transition: 'left 0.2s', boxShadow: '0 1px 3px rgba(0,0,0,0.2)',
                            }} />
                          </button>
                          <button onClick={() => { setEditingRule(rule); setShowRuleDialog(true); }}
                            className="btn btn-secondary btn-sm" style={{ padding: '5px 8px' }}>
                            <Edit3 size={11} />
                          </button>
                          <button onClick={() => handleDeleteRule(rule.id as string)}
                            className="btn btn-ghost btn-sm" style={{ padding: '5px 8px', color: 'var(--error)' }}>
                            <Trash2 size={11} />
                          </button>
                        </div>
                      </div>
                    </div>
                  </motion.div>
                );
              })}
            </div>
          )}
        </>
      )}

      {/* Alert Rule Dialog */}
      <AlertRuleDialog
        key={editingRule?.id || 'new'}
        open={showRuleDialog}
        onClose={() => { setShowRuleDialog(false); setEditingRule(null); }}
        onSave={handleSaveRule}
        editRule={editingRule}
        mcpServers={mcpServers}
        integrations={connectedIntegrations}
      />
    </div>
  );
}
