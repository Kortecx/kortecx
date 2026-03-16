'use client';

import { useState } from 'react';
import {
  Cable, Plus, Search, X, Check, ExternalLink, Trash2,
  ChevronDown, ChevronUp, Settings, Download, Star,
  Puzzle, Globe, Database, Cloud, MessageSquare, CreditCard,
  BarChart3, Activity, Phone, Mail, HardDrive, BookOpen,
  Ticket, Terminal, FileText, Image, Languages, Webhook,
  Eye, EyeOff, Shield, Package, Store, User, Github,
  Search as SearchIcon,
} from 'lucide-react';
import { INTEGRATION_CATALOG, MARKETPLACE_PLUGINS } from '@/lib/constants';
import type { IntegrationCategory } from '@/lib/types';

/* ── Icon resolver ──────────────────────────────────── */
const ICON_MAP: Record<string, React.ComponentType<{ size?: number; color?: string }>> = {
  MessageSquare, Github, Ticket, BookOpen, Database, HardDrive,
  Cloud, CreditCard, Phone, Mail, Search: SearchIcon, BarChart3,
  Activity, Webhook, Globe, Terminal, FileText, Image, Languages,
  Package, Store, Cable, Puzzle,
};

function ResolveIcon({ name, size = 14, color }: { name: string; size?: number; color?: string }) {
  const Icon = ICON_MAP[name] || Cable;
  return <Icon size={size} color={color} />;
}

/* ── Category meta ──────────────────────────────────── */
const CATEGORY_META: Record<IntegrationCategory, { label: string; color: string }> = {
  api:       { label: 'API',        color: '#2563EB' },
  app:       { label: 'App',        color: '#7C3AED' },
  tool:      { label: 'Tool',       color: '#059669' },
  database:  { label: 'Database',   color: '#D97706' },
  storage:   { label: 'Storage',    color: '#0EA5E9' },
  messaging: { label: 'Messaging',  color: '#EC4899' },
  analytics: { label: 'Analytics',  color: '#8B5CF6' },
};

/* ── Connected integration state ────────────────────── */
interface ConnectedIntegration {
  id: string;
  integrationId: string;
  name: string;
  status: 'active' | 'error' | 'expired';
  connectedAt: string;
}

/* ── Installed plugin state ─────────────────────────── */
interface InstalledPlugin {
  id: string;
  pluginId: string;
  name: string;
  source: 'personal' | 'marketplace';
  status: 'active' | 'disabled' | 'error';
  version: string;
  installedAt: string;
}

/* ── Personal plugin ────────────────────────────────── */
interface PersonalPlugin {
  id: string;
  name: string;
  description: string;
  version: string;
  category: string;
  capabilities: string;
}

/* ── Main Page ──────────────────────────────────────── */
export default function ConnectionsPage() {
  const [activeSection, setActiveSection] = useState<'integrations' | 'plugins'>('integrations');

  /* Integration state */
  const [search, setSearch] = useState('');
  const [catFilter, setCatFilter] = useState<IntegrationCategory | 'all'>('all');
  const [connectedIntegrations, setConnectedIntegrations] = useState<ConnectedIntegration[]>([]);
  const [connectingId, setConnectingId] = useState<string | null>(null);
  const [connectConfig, setConnectConfig] = useState<Record<string, string>>({});

  /* Plugin state */
  const [pluginSearch, setPluginSearch] = useState('');
  const [pluginTab, setPluginTab] = useState<'marketplace' | 'personal'>('marketplace');
  const [installedPlugins, setInstalledPlugins] = useState<InstalledPlugin[]>([]);
  const [showCreatePlugin, setShowCreatePlugin] = useState(false);
  const [newPlugin, setNewPlugin] = useState<PersonalPlugin>({
    id: '', name: '', description: '', version: '1.0.0', category: 'tool', capabilities: '',
  });

  /* Integration helpers */
  const filtered = INTEGRATION_CATALOG.filter(i => {
    if (search && !i.name.toLowerCase().includes(search.toLowerCase()) &&
      !i.description.toLowerCase().includes(search.toLowerCase())) return false;
    if (catFilter !== 'all' && i.category !== catFilter) return false;
    return true;
  });

  const isConnected = (integrationId: string) =>
    connectedIntegrations.some(c => c.integrationId === integrationId);

  const handleConnect = (integrationId: string) => {
    const integration = INTEGRATION_CATALOG.find(i => i.id === integrationId);
    if (!integration) return;
    const conn: ConnectedIntegration = {
      id: `conn-${Date.now()}`,
      integrationId,
      name: integration.name,
      status: 'active',
      connectedAt: new Date().toISOString(),
    };
    setConnectedIntegrations(prev => [...prev, conn]);
    setConnectingId(null);
    setConnectConfig({});
  };

  const handleDisconnect = (connId: string) => {
    setConnectedIntegrations(prev => prev.filter(c => c.id !== connId));
  };

  /* Plugin helpers */
  const filteredMarketplace = MARKETPLACE_PLUGINS.filter(p => {
    if (pluginSearch && !p.name.toLowerCase().includes(pluginSearch.toLowerCase()) &&
      !p.description.toLowerCase().includes(pluginSearch.toLowerCase())) return false;
    return true;
  });

  const isInstalled = (pluginId: string) =>
    installedPlugins.some(p => p.pluginId === pluginId);

  const handleInstallPlugin = (pluginId: string) => {
    const plugin = MARKETPLACE_PLUGINS.find(p => p.id === pluginId);
    if (!plugin) return;
    setInstalledPlugins(prev => [...prev, {
      id: `inst-${Date.now()}`,
      pluginId,
      name: plugin.name,
      source: 'marketplace',
      status: 'active',
      version: plugin.version,
      installedAt: new Date().toISOString(),
    }]);
  };

  const handleUninstallPlugin = (instId: string) => {
    setInstalledPlugins(prev => prev.filter(p => p.id !== instId));
  };

  const handleCreatePlugin = () => {
    if (!newPlugin.name.trim()) return;
    const plugin: InstalledPlugin = {
      id: `inst-${Date.now()}`,
      pluginId: `personal-${Date.now()}`,
      name: newPlugin.name,
      source: 'personal',
      status: 'active',
      version: newPlugin.version,
      installedAt: new Date().toISOString(),
    };
    setInstalledPlugins(prev => [...prev, plugin]);
    setNewPlugin({ id: '', name: '', description: '', version: '1.0.0', category: 'tool', capabilities: '' });
    setShowCreatePlugin(false);
  };

  const connectedCount = connectedIntegrations.length;
  const installedCount = installedPlugins.length;

  return (
    <div style={{ padding: 24, maxWidth: 1200, margin: '0 auto' }}>
      {/* Header */}
      <div style={{ marginBottom: 24 }}>
        <h1 style={{ fontSize: 20, fontWeight: 700, color: 'var(--text-1)', margin: 0, display: 'flex', alignItems: 'center', gap: 10 }}>
          <Cable size={20} /> Connections
        </h1>
        <p style={{ fontSize: 13, color: 'var(--text-3)', margin: '4px 0 0' }}>
          Manage external integrations and plugins for your workflow agents
        </p>
      </div>

      {/* Section tabs */}
      <div style={{ display: 'flex', gap: 4, marginBottom: 24, borderBottom: '1px solid var(--border)', paddingBottom: 0 }}>
        {([
          { id: 'integrations' as const, label: 'Integrations', icon: Cable, count: connectedCount },
          { id: 'plugins' as const, label: 'Plugins', icon: Puzzle, count: installedCount },
        ]).map(tab => (
          <button key={tab.id} onClick={() => setActiveSection(tab.id)}
            style={{
              display: 'flex', alignItems: 'center', gap: 8, padding: '10px 20px',
              background: 'none', border: 'none',
              borderBottom: `2px solid ${activeSection === tab.id ? 'var(--primary)' : 'transparent'}`,
              cursor: 'pointer', fontSize: 13, fontWeight: activeSection === tab.id ? 700 : 400,
              color: activeSection === tab.id ? 'var(--text-1)' : 'var(--text-3)',
              transition: 'all 0.12s',
            }}>
            <tab.icon size={14} />
            {tab.label}
            {tab.count > 0 && (
              <span style={{
                fontSize: 10, fontWeight: 700, padding: '1px 6px', borderRadius: 10,
                background: activeSection === tab.id ? 'var(--primary-dim)' : 'var(--bg-elevated)',
                color: activeSection === tab.id ? 'var(--primary-text)' : 'var(--text-4)',
              }}>{tab.count}</span>
            )}
          </button>
        ))}
      </div>

      {/* ── INTEGRATIONS SECTION ── */}
      {activeSection === 'integrations' && (
        <div>
          {/* Security notice */}
          <div style={{
            display: 'flex', alignItems: 'center', gap: 10,
            padding: '12px 16px', marginBottom: 20,
            background: 'rgba(37,99,235,0.05)',
            border: '1px solid rgba(37,99,235,0.15)',
            borderRadius: 6,
          }}>
            <Shield size={16} color="#2563EB" />
            <span style={{ fontSize: 12, color: '#2563EB' }}>
              Credentials are encrypted at rest. Connected integrations are available to all workflow steps.
            </span>
          </div>

          {/* Connected integrations */}
          {connectedIntegrations.length > 0 && (
            <div style={{ marginBottom: 28 }}>
              <h2 style={{ fontSize: 13, fontWeight: 700, color: 'var(--text-2)', marginBottom: 12,
                textTransform: 'uppercase', letterSpacing: '0.08em' }}>
                Connected ({connectedIntegrations.length})
              </h2>
              <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(280px, 1fr))', gap: 10 }}>
                {connectedIntegrations.map(conn => {
                  const catalog = INTEGRATION_CATALOG.find(i => i.id === conn.integrationId);
                  if (!catalog) return null;
                  const cm = CATEGORY_META[catalog.category];
                  return (
                    <div key={conn.id} className="card" style={{ padding: '14px 16px' }}>
                      <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                        <div style={{
                          width: 36, height: 36, borderRadius: 6,
                          background: `${catalog.color}12`, border: `1px solid ${catalog.color}30`,
                          display: 'flex', alignItems: 'center', justifyContent: 'center',
                        }}>
                          <ResolveIcon name={catalog.icon} size={16} color={catalog.color} />
                        </div>
                        <div style={{ flex: 1, minWidth: 0 }}>
                          <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                            <span style={{ fontSize: 13, fontWeight: 600, color: 'var(--text-1)' }}>{conn.name}</span>
                            <span style={{
                              fontSize: 9, fontWeight: 700, padding: '1px 5px', borderRadius: 3,
                              background: conn.status === 'active' ? 'rgba(5,150,105,0.1)' : 'rgba(220,38,38,0.1)',
                              color: conn.status === 'active' ? '#059669' : '#DC2626',
                              textTransform: 'uppercase',
                            }}>{conn.status}</span>
                          </div>
                          <div style={{ fontSize: 10, color: 'var(--text-4)', marginTop: 2 }}>
                            <span style={{ color: cm.color, fontWeight: 600 }}>{cm.label}</span> · Connected {new Date(conn.connectedAt).toLocaleDateString()}
                          </div>
                        </div>
                        <button onClick={() => handleDisconnect(conn.id)}
                          className="btn btn-ghost btn-icon btn-sm"
                          style={{ color: 'var(--text-4)' }} title="Disconnect">
                          <Trash2 size={13} />
                        </button>
                      </div>
                    </div>
                  );
                })}
              </div>
            </div>
          )}

          {/* Search + filter */}
          <div style={{ display: 'flex', gap: 12, marginBottom: 16 }}>
            <div style={{
              display: 'flex', alignItems: 'center', gap: 8, flex: 1,
              background: 'var(--bg-card)', border: '1px solid var(--border-md)',
              borderRadius: 6, padding: '8px 12px',
            }}>
              <Search size={14} color="var(--text-3)" />
              <input className="input" style={{ background: 'none', border: 'none', padding: 0, fontSize: 13 }}
                placeholder="Search integrations..." value={search}
                onChange={e => setSearch(e.target.value)} />
              {search && (
                <button onClick={() => setSearch('')}
                  style={{ background: 'none', border: 'none', cursor: 'pointer', color: 'var(--text-4)', display: 'flex', padding: 0 }}>
                  <X size={13} />
                </button>
              )}
            </div>
          </div>

          {/* Category pills */}
          <div style={{ display: 'flex', gap: 5, flexWrap: 'wrap', marginBottom: 16 }}>
            <button onClick={() => setCatFilter('all')} style={{
              padding: '4px 12px', borderRadius: 20, fontSize: 11, fontWeight: 600,
              border: '1px solid', cursor: 'pointer',
              background: catFilter === 'all' ? 'var(--primary-dim)' : 'transparent',
              borderColor: catFilter === 'all' ? 'var(--primary)' : 'var(--border)',
              color: catFilter === 'all' ? 'var(--primary-text)' : 'var(--text-3)',
            }}>All</button>
            {(Object.entries(CATEGORY_META) as [IntegrationCategory, { label: string; color: string }][]).map(([key, meta]) => (
              <button key={key} onClick={() => setCatFilter(catFilter === key ? 'all' : key)} style={{
                padding: '4px 12px', borderRadius: 20, fontSize: 11, fontWeight: 600,
                border: '1px solid', cursor: 'pointer',
                background: catFilter === key ? `${meta.color}15` : 'transparent',
                borderColor: catFilter === key ? meta.color : 'var(--border)',
                color: catFilter === key ? meta.color : 'var(--text-3)',
                transition: 'all 0.1s',
              }}>{meta.label}</button>
            ))}
          </div>

          {/* Integration catalog grid */}
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(300px, 1fr))', gap: 10 }}>
            {filtered.map(integration => {
              const connected = isConnected(integration.id);
              const cm = CATEGORY_META[integration.category];
              return (
                <div key={integration.id} className="card" style={{ padding: '16px 18px' }}>
                  <div style={{ display: 'flex', alignItems: 'flex-start', gap: 12 }}>
                    <div style={{
                      width: 40, height: 40, borderRadius: 8, flexShrink: 0,
                      background: `${integration.color}12`, border: `1px solid ${integration.color}25`,
                      display: 'flex', alignItems: 'center', justifyContent: 'center',
                    }}>
                      <ResolveIcon name={integration.icon} size={18} color={integration.color} />
                    </div>
                    <div style={{ flex: 1, minWidth: 0 }}>
                      <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                        <span style={{ fontSize: 14, fontWeight: 600, color: 'var(--text-1)' }}>{integration.name}</span>
                        <span style={{
                          fontSize: 9, fontWeight: 700, padding: '1px 6px', borderRadius: 3,
                          background: `${cm.color}12`, color: cm.color,
                          textTransform: 'uppercase', letterSpacing: '0.04em',
                        }}>{cm.label}</span>
                        {connected && (
                          <span style={{
                            fontSize: 9, fontWeight: 700, padding: '1px 5px', borderRadius: 3,
                            background: 'rgba(5,150,105,0.1)', color: '#059669',
                          }}>CONNECTED</span>
                        )}
                      </div>
                      <div style={{ fontSize: 12, color: 'var(--text-3)', marginTop: 4, lineHeight: 1.4 }}>
                        {integration.description}
                      </div>
                      <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginTop: 8 }}>
                        <span style={{
                          fontSize: 10, color: 'var(--text-4)',
                          display: 'flex', alignItems: 'center', gap: 3,
                        }}>
                          <Shield size={9} /> {integration.authType.replace('_', ' ')}
                        </span>
                      </div>
                    </div>
                  </div>
                  <div style={{ display: 'flex', gap: 6, marginTop: 12, justifyContent: 'flex-end' }}>
                    {connected ? (
                      <button className="btn btn-ghost btn-sm" style={{ fontSize: 11 }}>
                        <Settings size={11} /> Configure
                      </button>
                    ) : (
                      <button className="btn btn-primary btn-sm" style={{ fontSize: 11 }}
                        onClick={() => setConnectingId(integration.id)}>
                        <Plus size={11} /> Connect
                      </button>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
          {filtered.length === 0 && (
            <div style={{ textAlign: 'center', padding: '40px 20px', color: 'var(--text-3)', fontSize: 13 }}>
              No integrations match your search.
            </div>
          )}
        </div>
      )}

      {/* ── PLUGINS SECTION ── */}
      {activeSection === 'plugins' && (
        <div>
          {/* Plugin sub-tabs */}
          <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 20 }}>
            <div style={{ display: 'flex', gap: 4 }}>
              {([
                { id: 'marketplace' as const, label: 'Marketplace', icon: Store },
                { id: 'personal' as const, label: 'My Plugins', icon: User },
              ]).map(tab => (
                <button key={tab.id} onClick={() => setPluginTab(tab.id)} style={{
                  display: 'flex', alignItems: 'center', gap: 6,
                  padding: '6px 14px', borderRadius: 6, fontSize: 12, fontWeight: pluginTab === tab.id ? 600 : 400,
                  border: `1px solid ${pluginTab === tab.id ? 'var(--primary)' : 'var(--border)'}`,
                  background: pluginTab === tab.id ? 'var(--primary-dim)' : 'transparent',
                  color: pluginTab === tab.id ? 'var(--primary-text)' : 'var(--text-3)',
                  cursor: 'pointer', transition: 'all 0.12s',
                }}>
                  <tab.icon size={13} />
                  {tab.label}
                </button>
              ))}
            </div>
            {pluginTab === 'personal' && (
              <button className="btn btn-primary btn-sm" onClick={() => setShowCreatePlugin(true)}>
                <Plus size={13} /> Create Plugin
              </button>
            )}
          </div>

          {/* Installed plugins */}
          {installedPlugins.length > 0 && (
            <div style={{ marginBottom: 24 }}>
              <h2 style={{ fontSize: 13, fontWeight: 700, color: 'var(--text-2)', marginBottom: 12,
                textTransform: 'uppercase', letterSpacing: '0.08em' }}>
                Installed ({installedPlugins.length})
              </h2>
              <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(260px, 1fr))', gap: 10 }}>
                {installedPlugins
                  .filter(p => pluginTab === 'personal' ? p.source === 'personal' : p.source === 'marketplace')
                  .map(inst => {
                    const mpPlugin = MARKETPLACE_PLUGINS.find(p => p.id === inst.pluginId);
                    return (
                      <div key={inst.id} className="card" style={{ padding: '12px 14px' }}>
                        <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                          <div style={{
                            width: 32, height: 32, borderRadius: 6,
                            background: mpPlugin ? `${mpPlugin.color}12` : 'var(--bg-elevated)',
                            border: `1px solid ${mpPlugin ? `${mpPlugin.color}25` : 'var(--border)'}`,
                            display: 'flex', alignItems: 'center', justifyContent: 'center',
                          }}>
                            {mpPlugin ? (
                              <ResolveIcon name={mpPlugin.icon} size={14} color={mpPlugin.color} />
                            ) : (
                              <Puzzle size={14} color="var(--text-3)" />
                            )}
                          </div>
                          <div style={{ flex: 1, minWidth: 0 }}>
                            <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                              <span style={{ fontSize: 12, fontWeight: 600, color: 'var(--text-1)' }}>{inst.name}</span>
                              <span style={{ fontSize: 9, color: 'var(--text-4)' }}>v{inst.version}</span>
                            </div>
                            <div style={{ fontSize: 10, color: 'var(--text-4)', marginTop: 1 }}>
                              {inst.source === 'marketplace' ? 'Marketplace' : 'Personal'} · {inst.status}
                            </div>
                          </div>
                          <button onClick={() => handleUninstallPlugin(inst.id)}
                            className="btn btn-ghost btn-icon btn-sm"
                            style={{ color: 'var(--text-4)' }} title="Uninstall">
                            <Trash2 size={12} />
                          </button>
                        </div>
                      </div>
                    );
                  })}
              </div>
            </div>
          )}

          {/* Marketplace browser */}
          {pluginTab === 'marketplace' && (
            <div>
              <div style={{
                display: 'flex', alignItems: 'center', gap: 8,
                background: 'var(--bg-card)', border: '1px solid var(--border-md)',
                borderRadius: 6, padding: '8px 12px', marginBottom: 16,
              }}>
                <Search size={14} color="var(--text-3)" />
                <input className="input" style={{ background: 'none', border: 'none', padding: 0, fontSize: 13 }}
                  placeholder="Search marketplace plugins..." value={pluginSearch}
                  onChange={e => setPluginSearch(e.target.value)} />
                {pluginSearch && (
                  <button onClick={() => setPluginSearch('')}
                    style={{ background: 'none', border: 'none', cursor: 'pointer', color: 'var(--text-4)', display: 'flex', padding: 0 }}>
                    <X size={13} />
                  </button>
                )}
              </div>

              <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(320px, 1fr))', gap: 12 }}>
                {filteredMarketplace.map(plugin => {
                  const installed = isInstalled(plugin.id);
                  return (
                    <div key={plugin.id} className="card" style={{ padding: '18px 20px' }}>
                      <div style={{ display: 'flex', alignItems: 'flex-start', gap: 12 }}>
                        <div style={{
                          width: 44, height: 44, borderRadius: 8, flexShrink: 0,
                          background: `${plugin.color}12`, border: `1px solid ${plugin.color}25`,
                          display: 'flex', alignItems: 'center', justifyContent: 'center',
                        }}>
                          <ResolveIcon name={plugin.icon} size={20} color={plugin.color} />
                        </div>
                        <div style={{ flex: 1, minWidth: 0 }}>
                          <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                            <span style={{ fontSize: 14, fontWeight: 600, color: 'var(--text-1)' }}>{plugin.name}</span>
                            <span className="mono" style={{ fontSize: 10, color: 'var(--text-4)' }}>v{plugin.version}</span>
                          </div>
                          <div style={{ fontSize: 11, color: 'var(--text-4)', marginTop: 2 }}>
                            by {plugin.author}
                          </div>
                          <div style={{ fontSize: 12, color: 'var(--text-3)', marginTop: 6, lineHeight: 1.4 }}>
                            {plugin.description}
                          </div>
                          <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginTop: 8 }}>
                            <span style={{ display: 'flex', alignItems: 'center', gap: 3, fontSize: 10, color: 'var(--text-4)' }}>
                              <Download size={9} /> {plugin.downloads.toLocaleString()}
                            </span>
                            <span style={{ display: 'flex', alignItems: 'center', gap: 3, fontSize: 10, color: '#F59E0B' }}>
                              <Star size={9} /> {plugin.rating}
                            </span>
                            <span style={{
                              fontSize: 9, padding: '1px 6px', borderRadius: 3,
                              background: 'var(--bg-elevated)', color: 'var(--text-4)',
                              fontWeight: 600, textTransform: 'uppercase',
                            }}>{plugin.category}</span>
                          </div>
                          <div style={{ display: 'flex', gap: 4, flexWrap: 'wrap', marginTop: 6 }}>
                            {plugin.capabilities.map(cap => (
                              <span key={cap} style={{
                                fontSize: 9, padding: '1px 6px', borderRadius: 10,
                                background: `${plugin.color}08`, border: `1px solid ${plugin.color}20`,
                                color: plugin.color, fontWeight: 500,
                              }}>{cap}</span>
                            ))}
                          </div>
                        </div>
                      </div>
                      <div style={{ display: 'flex', justifyContent: 'flex-end', marginTop: 12 }}>
                        {installed ? (
                          <span style={{
                            display: 'flex', alignItems: 'center', gap: 4,
                            fontSize: 11, color: '#059669', fontWeight: 600,
                          }}>
                            <Check size={12} /> Installed
                          </span>
                        ) : (
                          <button className="btn btn-primary btn-sm" style={{ fontSize: 11 }}
                            onClick={() => handleInstallPlugin(plugin.id)}>
                            <Download size={11} /> Install
                          </button>
                        )}
                      </div>
                    </div>
                  );
                })}
              </div>
              {filteredMarketplace.length === 0 && (
                <div style={{ textAlign: 'center', padding: '40px 20px', color: 'var(--text-3)', fontSize: 13 }}>
                  No plugins match your search.
                </div>
              )}
            </div>
          )}

          {/* Personal plugins */}
          {pluginTab === 'personal' && (
            <div>
              {installedPlugins.filter(p => p.source === 'personal').length === 0 && !showCreatePlugin && (
                <div style={{
                  textAlign: 'center', padding: '60px 20px',
                  border: '1px dashed var(--border-md)', borderRadius: 8,
                }}>
                  <Puzzle size={28} color="var(--text-3)" style={{ margin: '0 auto 12px' }} />
                  <div style={{ fontSize: 14, fontWeight: 600, color: 'var(--text-2)', marginBottom: 6 }}>No personal plugins yet</div>
                  <div style={{ fontSize: 13, color: 'var(--text-3)', marginBottom: 16, maxWidth: 400, margin: '0 auto 16px' }}>
                    Create custom plugins to extend your workflow agents with specialized capabilities.
                  </div>
                  <button className="btn btn-primary" onClick={() => setShowCreatePlugin(true)}>
                    <Plus size={14} /> Create Your First Plugin
                  </button>
                </div>
              )}
            </div>
          )}
        </div>
      )}

      {/* ── Connect Integration Modal ── */}
      {connectingId && (() => {
        const integration = INTEGRATION_CATALOG.find(i => i.id === connectingId);
        if (!integration) return null;
        return (
          <div style={{
            position: 'fixed', inset: 0, background: 'rgba(0,0,0,0.4)',
            display: 'flex', alignItems: 'center', justifyContent: 'center', zIndex: 100,
          }}>
            <div style={{
              background: 'var(--bg-surface)', borderRadius: 8,
              padding: 24, width: 440, maxWidth: '90vw',
              boxShadow: '0 20px 60px rgba(0,0,0,0.15)',
            }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 20 }}>
                <div style={{
                  width: 40, height: 40, borderRadius: 8,
                  background: `${integration.color}12`, border: `1px solid ${integration.color}25`,
                  display: 'flex', alignItems: 'center', justifyContent: 'center',
                }}>
                  <ResolveIcon name={integration.icon} size={18} color={integration.color} />
                </div>
                <div>
                  <h3 style={{ fontSize: 16, fontWeight: 600, color: 'var(--text-1)', margin: 0 }}>
                    Connect {integration.name}
                  </h3>
                  <div style={{ fontSize: 12, color: 'var(--text-3)', marginTop: 2 }}>
                    Authentication: {integration.authType.replace('_', ' ')}
                  </div>
                </div>
              </div>

              {integration.authType === 'api_key' && (
                <div style={{ marginBottom: 16 }}>
                  <label style={{ fontSize: 12, fontWeight: 500, color: 'var(--text-2)', display: 'block', marginBottom: 4 }}>
                    API Key
                  </label>
                  <input className="input" type="password" placeholder="Enter your API key..."
                    style={{ width: '100%' }}
                    value={connectConfig.apiKey || ''}
                    onChange={e => setConnectConfig(prev => ({ ...prev, apiKey: e.target.value }))} />
                </div>
              )}
              {integration.authType === 'bearer' && (
                <div style={{ marginBottom: 16 }}>
                  <label style={{ fontSize: 12, fontWeight: 500, color: 'var(--text-2)', display: 'block', marginBottom: 4 }}>
                    Access Token
                  </label>
                  <input className="input" type="password" placeholder="Enter your access token..."
                    style={{ width: '100%' }}
                    value={connectConfig.token || ''}
                    onChange={e => setConnectConfig(prev => ({ ...prev, token: e.target.value }))} />
                </div>
              )}
              {integration.authType === 'basic' && (
                <>
                  <div style={{ marginBottom: 12 }}>
                    <label style={{ fontSize: 12, fontWeight: 500, color: 'var(--text-2)', display: 'block', marginBottom: 4 }}>
                      Host / URL
                    </label>
                    <input className="input" placeholder="e.g. localhost:5432"
                      style={{ width: '100%' }}
                      value={connectConfig.host || ''}
                      onChange={e => setConnectConfig(prev => ({ ...prev, host: e.target.value }))} />
                  </div>
                  <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 12, marginBottom: 16 }}>
                    <div>
                      <label style={{ fontSize: 12, fontWeight: 500, color: 'var(--text-2)', display: 'block', marginBottom: 4 }}>
                        Username
                      </label>
                      <input className="input" placeholder="Username"
                        style={{ width: '100%' }}
                        value={connectConfig.username || ''}
                        onChange={e => setConnectConfig(prev => ({ ...prev, username: e.target.value }))} />
                    </div>
                    <div>
                      <label style={{ fontSize: 12, fontWeight: 500, color: 'var(--text-2)', display: 'block', marginBottom: 4 }}>
                        Password
                      </label>
                      <input className="input" type="password" placeholder="Password"
                        style={{ width: '100%' }}
                        value={connectConfig.password || ''}
                        onChange={e => setConnectConfig(prev => ({ ...prev, password: e.target.value }))} />
                    </div>
                  </div>
                </>
              )}
              {integration.authType === 'oauth2' && (
                <div style={{
                  padding: '16px', background: 'var(--bg)', border: '1px solid var(--border)',
                  borderRadius: 6, marginBottom: 16, textAlign: 'center',
                }}>
                  <div style={{ fontSize: 12, color: 'var(--text-3)', marginBottom: 8 }}>
                    You will be redirected to {integration.name} to authorize access.
                  </div>
                  <button className="btn btn-secondary btn-sm">
                    <ExternalLink size={12} /> Authorize with {integration.name}
                  </button>
                </div>
              )}

              <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
                <button className="btn btn-secondary btn-sm" onClick={() => { setConnectingId(null); setConnectConfig({}); }}>
                  Cancel
                </button>
                <button className="btn btn-primary btn-sm" onClick={() => handleConnect(integration.id)}>
                  <Check size={12} /> Connect
                </button>
              </div>
            </div>
          </div>
        );
      })()}

      {/* ── Create Plugin Modal ── */}
      {showCreatePlugin && (
        <div style={{
          position: 'fixed', inset: 0, background: 'rgba(0,0,0,0.4)',
          display: 'flex', alignItems: 'center', justifyContent: 'center', zIndex: 100,
        }}>
          <div style={{
            background: 'var(--bg-surface)', borderRadius: 8,
            padding: 24, width: 480, maxWidth: '90vw',
            boxShadow: '0 20px 60px rgba(0,0,0,0.15)',
          }}>
            <h3 style={{ fontSize: 16, fontWeight: 600, color: 'var(--text-1)', margin: '0 0 20px' }}>
              Create Personal Plugin
            </h3>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
              <div>
                <label style={{ fontSize: 12, fontWeight: 500, color: 'var(--text-2)', display: 'block', marginBottom: 4 }}>
                  Plugin Name <span style={{ color: 'var(--error)' }}>*</span>
                </label>
                <input className="input" placeholder="My Custom Plugin" style={{ width: '100%' }}
                  value={newPlugin.name} onChange={e => setNewPlugin(prev => ({ ...prev, name: e.target.value }))} />
              </div>
              <div>
                <label style={{ fontSize: 12, fontWeight: 500, color: 'var(--text-2)', display: 'block', marginBottom: 4 }}>
                  Description
                </label>
                <textarea className="textarea" placeholder="What does this plugin do?" style={{ minHeight: 60, width: '100%' }}
                  value={newPlugin.description} onChange={e => setNewPlugin(prev => ({ ...prev, description: e.target.value }))} />
              </div>
              <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 12 }}>
                <div>
                  <label style={{ fontSize: 12, fontWeight: 500, color: 'var(--text-2)', display: 'block', marginBottom: 4 }}>
                    Version
                  </label>
                  <input className="input" placeholder="1.0.0" style={{ width: '100%' }}
                    value={newPlugin.version} onChange={e => setNewPlugin(prev => ({ ...prev, version: e.target.value }))} />
                </div>
                <div>
                  <label style={{ fontSize: 12, fontWeight: 500, color: 'var(--text-2)', display: 'block', marginBottom: 4 }}>
                    Category
                  </label>
                  <select className="input" style={{ width: '100%' }}
                    value={newPlugin.category} onChange={e => setNewPlugin(prev => ({ ...prev, category: e.target.value }))}>
                    <option value="tool">Tool</option>
                    <option value="data">Data</option>
                    <option value="analytics">Analytics</option>
                    <option value="creative">Creative</option>
                    <option value="communication">Communication</option>
                    <option value="language">Language</option>
                  </select>
                </div>
              </div>
              <div>
                <label style={{ fontSize: 12, fontWeight: 500, color: 'var(--text-2)', display: 'block', marginBottom: 4 }}>
                  Capabilities (comma-separated)
                </label>
                <input className="input" placeholder="e.g. parse, transform, validate" style={{ width: '100%' }}
                  value={newPlugin.capabilities} onChange={e => setNewPlugin(prev => ({ ...prev, capabilities: e.target.value }))} />
              </div>
            </div>
            <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end', marginTop: 20 }}>
              <button className="btn btn-secondary btn-sm" onClick={() => setShowCreatePlugin(false)}>
                Cancel
              </button>
              <button className="btn btn-primary btn-sm" onClick={handleCreatePlugin} disabled={!newPlugin.name.trim()}>
                <Plus size={12} /> Create Plugin
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
