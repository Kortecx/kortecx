'use client';

import { useState, useEffect, useCallback, Suspense } from 'react';
import { useSearchParams } from 'next/navigation';
import {
  Cable, Plus, Search, X, Check, ExternalLink, Trash2,
  ChevronDown, ChevronUp, Settings, Download, Star,
  Puzzle, Globe, Database, Cloud, MessageSquare, CreditCard,
  BarChart3, Activity, Phone, Mail, HardDrive, BookOpen,
  Ticket, Terminal, FileText, Image, Languages, Webhook,
  Eye, EyeOff, Shield, Package, Store, User, Github,
  Search as SearchIcon, Video, MessageCircle, AtSign,
  Send, Code, Camera, Target, HelpCircle, Headphones,
  TrendingUp, Zap, PieChart, Snowflake, LayoutGrid,
  Flame, Twitter, Linkedin, Facebook, Instagram, Youtube,
  Filter,
} from 'lucide-react';
import { INTEGRATION_CATALOG, MARKETPLACE_PLUGINS } from '@/lib/constants';
import type { IntegrationCategory } from '@/lib/types';

/* ── Icon resolver ──────────────────────────────────── */
const ICON_MAP: Record<string, React.ComponentType<{ size?: number; color?: string }>> = {
  MessageSquare, Github, Ticket, BookOpen, Database, HardDrive,
  Cloud, CreditCard, Phone, Mail, Search: SearchIcon, BarChart3,
  Activity, Webhook, Globe, Terminal, FileText, Image, Languages,
  Package, Store, Cable, Puzzle, Video, MessageCircle, AtSign,
  Send, Code, Camera, Target, HelpCircle, Headphones, TrendingUp,
  Zap, PieChart, Snowflake, Eye, LayoutGrid, Flame,
  Twitter, Linkedin, Facebook, Instagram, Youtube, Filter,
};

function ResolveIcon({ name, size = 14, color }: { name: string; size?: number; color?: string }) {
  const Icon = ICON_MAP[name] || Cable;
  return <Icon size={size} color={color} />;
}

/* ── Category meta ──────────────────────────────────── */
const CATEGORY_META: Record<IntegrationCategory, { label: string; color: string }> = {
  social:         { label: 'Social Media',     color: '#E1306C' },
  crm:            { label: 'CRM & Marketing',  color: '#FF7A59' },
  data_analytics: { label: 'Data & Analytics', color: '#7856FF' },
  messaging:      { label: 'Messaging',        color: '#EC4899' },
  api:            { label: 'API',              color: '#2563EB' },
  app:            { label: 'App',              color: '#7C3AED' },
  tool:           { label: 'Tool',             color: '#059669' },
  database:       { label: 'Database',         color: '#D97706' },
  storage:        { label: 'Storage',          color: '#0EA5E9' },
  analytics:      { label: 'Analytics',        color: '#8B5CF6' },
};

/* ── Capability labels ─────────────────────────────── */
const CAPABILITY_META: Record<string, { label: string; color: string }> = {
  consume:  { label: 'Consume',  color: '#2563EB' },
  generate: { label: 'Generate', color: '#7C3AED' },
  publish:  { label: 'Publish',  color: '#059669' },
  schedule: { label: 'Schedule', color: '#D97706' },
  report:   { label: 'Report',   color: '#0EA5E9' },
  execute:  { label: 'Execute',  color: '#DC2626' },
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

/* ── OAuth-connected social platforms ──────────────── */
interface OAuthConnection {
  id: string;
  platform: string;
  platformUsername: string;
  platformAvatar?: string;
  status: string;
  isExpired: boolean;
  createdAt: string;
}

/* ── Main Page ──────────────────────────────────────── */
export default function ConnectionsPage() {
  return (
    <Suspense>
      <ConnectionsPageInner />
    </Suspense>
  );
}

function ConnectionsPageInner() {
  const searchParams = useSearchParams();
  const [activeSection, setActiveSection] = useState<'integrations' | 'plugins'>('integrations');

  /* OAuth notification banner */
  const [oauthNotice, setOauthNotice] = useState<{ type: 'success' | 'error'; message: string } | null>(null);

  /* OAuth social connections (fetched from DB) */
  const [oauthConnections, setOauthConnections] = useState<OAuthConnection[]>([]);

  /* Configure modal state */
  const [configuringId, setConfiguringId] = useState<string | null>(null);
  const [configData, setConfigData] = useState<{
    platformUsername: string; platformAvatar?: string; scopes: string[];
    permissions: Record<string, boolean>; status: string; isExpired: boolean;
    hasRefreshToken: boolean; tokenExpiresAt?: string; lastUsedAt?: string;
    lastRefreshedAt?: string; connectedAt?: string;
  } | null>(null);
  const [configLoading, setConfigLoading] = useState(false);
  const [configRefreshing, setConfigRefreshing] = useState(false);
  const [permSaving, setPermSaving] = useState(false);

  /* OAuth credential state (client ID/secret stored in DB) */
  const [savedCredentials, setSavedCredentials] = useState<Record<string, { clientIdMasked: string; clientIdPrefix: string }>>({});
  const [credClientId, setCredClientId] = useState('');
  const [credClientSecret, setCredClientSecret] = useState('');
  const [credSaving, setCredSaving] = useState(false);
  const [credSaved, setCredSaved] = useState(false);
  const [showCredSecrets, setShowCredSecrets] = useState(false);

  /* Integration state */
  const [search, setSearch] = useState('');
  const [catFilter, setCatFilter] = useState<IntegrationCategory | 'all'>('all');
  const [connectedIntegrations, setConnectedIntegrations] = useState<ConnectedIntegration[]>([]);
  const [connectingId, setConnectingId] = useState<string | null>(null);
  const [connectConfig, setConnectConfig] = useState<Record<string, string>>({});

  /* Fetch OAuth connections from DB */
  const fetchOAuthConnections = useCallback(async () => {
    try {
      const res = await fetch('/api/oauth/connections');
      if (res.ok) {
        const data = await res.json();
        setOauthConnections(data.connections || []);
      }
    } catch { /* ignore */ }
  }, []);

  /* Fetch stored OAuth credentials */
  const fetchCredentials = useCallback(async () => {
    try {
      const res = await fetch('/api/oauth/credentials');
      if (res.ok) {
        const data = await res.json();
        const map: Record<string, { clientIdMasked: string; clientIdPrefix: string }> = {};
        for (const c of data.credentials || []) {
          map[c.platform] = { clientIdMasked: c.clientIdMasked, clientIdPrefix: c.clientIdPrefix };
        }
        setSavedCredentials(map);
      }
    } catch { /* ignore */ }
  }, []);

  /* Save OAuth credentials to DB */
  const handleSaveCredentials = async (platform: string) => {
    if (!credClientId.trim() || !credClientSecret.trim()) return;
    setCredSaving(true);
    try {
      const res = await fetch('/api/oauth/credentials', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ platform, clientId: credClientId.trim(), clientSecret: credClientSecret.trim() }),
      });
      if (res.ok) {
        const data = await res.json();
        setSavedCredentials(prev => ({
          ...prev,
          [platform]: { clientIdMasked: data.clientIdMasked, clientIdPrefix: credClientId.slice(0, 6) },
        }));
        setCredSaved(true);
        setTimeout(() => setCredSaved(false), 3000);
      }
    } catch { /* ignore */ }
    setCredSaving(false);
  };

  /* Open configure modal — fetch connection details */
  const handleOpenConfigure = async (platformId: string) => {
    setConfiguringId(platformId);
    setConfigLoading(true);
    setConfigData(null);
    try {
      const res = await fetch(`/api/oauth/connections/configure?platform=${platformId}`);
      if (res.ok) {
        const data = await res.json();
        setConfigData(data.connection);
      }
    } catch { /* ignore */ }
    setConfigLoading(false);
  };

  /* Toggle a permission */
  const handleTogglePermission = async (platform: string, key: string, value: boolean) => {
    if (!configData) return;
    const updated = { ...configData.permissions, [key]: value };
    setConfigData(prev => prev ? { ...prev, permissions: updated } : null);
    setPermSaving(true);
    try {
      await fetch('/api/oauth/connections/configure', {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ platform, action: 'update_permissions', permissions: updated }),
      });
    } catch { /* ignore */ }
    setPermSaving(false);
  };

  /* Refresh token */
  const handleRefreshToken = async (platform: string) => {
    setConfigRefreshing(true);
    try {
      const res = await fetch('/api/oauth/connections/configure', {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ platform, action: 'refresh_token' }),
      });
      if (res.ok) {
        const data = await res.json();
        setConfigData(prev => prev ? {
          ...prev,
          status: data.status || 'active',
          isExpired: false,
          tokenExpiresAt: data.tokenExpiresAt,
          lastRefreshedAt: new Date().toISOString(),
        } : null);
        setOauthNotice({ type: 'success', message: `Token refreshed for ${platform}` });
        fetchOAuthConnections();
      } else {
        const err = await res.json();
        setOauthNotice({ type: 'error', message: err.error || 'Refresh failed' });
      }
    } catch {
      setOauthNotice({ type: 'error', message: 'Token refresh failed' });
    }
    setConfigRefreshing(false);
  };

  useEffect(() => {
    fetchOAuthConnections();
    fetchCredentials();
  }, [fetchOAuthConnections, fetchCredentials]);

  /* Handle OAuth callback — via query params (fallback) or postMessage from popup */
  useEffect(() => {
    const connected = searchParams.get('connected');
    const username = searchParams.get('username');
    const error = searchParams.get('error');
    const platform = searchParams.get('platform');

    if (connected) {
      setOauthNotice({
        type: 'success',
        message: `Successfully connected to ${connected}${username ? ` as @${username}` : ''}`,
      });
      fetchOAuthConnections();
      setConnectingId(null);
      window.history.replaceState({}, '', '/providers/connections');
    } else if (error) {
      setOauthNotice({
        type: 'error',
        message: `${platform ? `${platform}: ` : ''}${error}`,
      });
      window.history.replaceState({}, '', '/providers/connections');
    }
  }, [searchParams, fetchOAuthConnections]);

  /* Listen for postMessage from OAuth popup window */
  useEffect(() => {
    const handleMessage = (event: MessageEvent) => {
      if (event.origin !== window.location.origin) return;
      const data = event.data;
      if (data?.type === 'oauth_success') {
        setOauthNotice({
          type: 'success',
          message: `Successfully connected to ${data.platform}${data.username ? ` as @${data.username}` : ''}`,
        });
        fetchOAuthConnections();
        setConnectingId(null);
      } else if (data?.type === 'oauth_error') {
        setOauthNotice({
          type: 'error',
          message: `${data.platform ? `${data.platform}: ` : ''}${data.error}`,
        });
      }
    };
    window.addEventListener('message', handleMessage);
    return () => window.removeEventListener('message', handleMessage);
  }, [fetchOAuthConnections]);

  /* Auto-dismiss notice */
  useEffect(() => {
    if (oauthNotice) {
      const timer = setTimeout(() => setOauthNotice(null), 8000);
      return () => clearTimeout(timer);
    }
  }, [oauthNotice]);

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
    connectedIntegrations.some(c => c.integrationId === integrationId) ||
    oauthConnections.some(c => c.platform === integrationId && c.status === 'active');

  /** Initiate OAuth flow — opens platform consent screen in a new browser tab. */
  const handleOAuthConnect = (platformId: string) => {
    window.open(`/api/oauth/${platformId}/authorize`, '_blank');
  };

  /** Disconnect an OAuth platform. */
  const handleOAuthDisconnect = async (platform: string) => {
    try {
      await fetch(`/api/oauth/connections?platform=${platform}`, { method: 'DELETE' });
      setOauthConnections(prev => prev.filter(c => c.platform !== platform));
    } catch { /* ignore */ }
  };

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

  const connectedCount = connectedIntegrations.length + oauthConnections.length;
  const installedCount = installedPlugins.length;

  return (
    <div style={{ padding: 24, maxWidth: 1200, margin: '0 auto' }}>
      {/* OAuth callback notification */}
      {oauthNotice && (
        <div style={{
          display: 'flex', alignItems: 'center', gap: 10,
          padding: '12px 16px', marginBottom: 16,
          background: oauthNotice.type === 'success' ? 'rgba(5,150,105,0.08)' : 'rgba(220,38,38,0.08)',
          border: `1px solid ${oauthNotice.type === 'success' ? 'rgba(5,150,105,0.2)' : 'rgba(220,38,38,0.2)'}`,
          borderRadius: 6,
        }}>
          {oauthNotice.type === 'success' ? <Check size={16} color="#059669" /> : <X size={16} color="#DC2626" />}
          <span style={{ flex: 1, fontSize: 13, color: oauthNotice.type === 'success' ? '#059669' : '#DC2626', fontWeight: 500 }}>
            {oauthNotice.message}
          </span>
          <button onClick={() => setOauthNotice(null)}
            style={{ background: 'none', border: 'none', cursor: 'pointer', color: 'var(--text-4)', display: 'flex', padding: 0 }}>
            <X size={14} />
          </button>
        </div>
      )}

      {/* Header */}
      <div style={{ marginBottom: 24 }}>
        <h1 style={{ fontSize: 20, fontWeight: 700, color: 'var(--text-1)', margin: 0, display: 'flex', alignItems: 'center', gap: 10 }}>
          <Cable size={20} /> Connections
        </h1>
        <p style={{ fontSize: 13, color: 'var(--text-3)', margin: '4px 0 0' }}>
          Manage external integrations and plugins for your workflow agents
        </p>
      </div>

      {/* Connected Social Accounts */}
      {oauthConnections.length > 0 && (
        <div style={{ marginBottom: 24 }}>
          <h2 style={{ fontSize: 13, fontWeight: 700, color: 'var(--text-2)', marginBottom: 12,
            textTransform: 'uppercase', letterSpacing: '0.08em' }}>
            Connected Accounts ({oauthConnections.length})
          </h2>
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(280px, 1fr))', gap: 10 }}>
            {oauthConnections.map(conn => {
              const catalogEntry = INTEGRATION_CATALOG.find(i => i.id === conn.platform);
              const color = catalogEntry?.color || '#6B7280';
              return (
                <div key={conn.id} className="card" style={{ padding: '14px 16px' }}>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                    <div style={{
                      width: 36, height: 36, borderRadius: '50%',
                      background: `${color}12`, border: `1px solid ${color}30`,
                      display: 'flex', alignItems: 'center', justifyContent: 'center',
                      overflow: 'hidden',
                    }}>
                      {conn.platformAvatar ? (
                        <img src={conn.platformAvatar} alt="" width={36} height={36} style={{ borderRadius: '50%', objectFit: 'cover' }} />
                      ) : catalogEntry ? (
                        <ResolveIcon name={catalogEntry.icon} size={16} color={color} />
                      ) : (
                        <Cable size={16} color={color} />
                      )}
                    </div>
                    <div style={{ flex: 1, minWidth: 0 }}>
                      <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                        <span style={{ fontSize: 13, fontWeight: 600, color: 'var(--text-1)' }}>
                          {catalogEntry?.name || conn.platform}
                        </span>
                        <span style={{
                          fontSize: 9, fontWeight: 700, padding: '1px 5px', borderRadius: 3,
                          background: conn.status === 'active' ? 'rgba(5,150,105,0.1)' : conn.isExpired ? 'rgba(217,119,6,0.1)' : 'rgba(220,38,38,0.1)',
                          color: conn.status === 'active' ? '#059669' : conn.isExpired ? '#D97706' : '#DC2626',
                          textTransform: 'uppercase',
                        }}>{conn.isExpired ? 'EXPIRED' : conn.status}</span>
                      </div>
                      <div style={{ fontSize: 11, color: 'var(--text-3)', marginTop: 2 }}>
                        @{conn.platformUsername}
                      </div>
                    </div>
                    {conn.isExpired && (
                      <button onClick={() => handleOAuthConnect(conn.platform)}
                        className="btn btn-ghost btn-sm"
                        style={{ fontSize: 10, color: '#D97706' }} title="Reconnect">
                        <ExternalLink size={11} /> Reconnect
                      </button>
                    )}
                    <button onClick={() => handleOAuthDisconnect(conn.platform)}
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

          {/* Search + Category filter */}
          <div style={{ display: 'flex', gap: 12, marginBottom: 16, alignItems: 'center' }}>
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
            {/* Category dropdown filter */}
            <div style={{ position: 'relative' }}>
              <select
                value={catFilter}
                onChange={e => setCatFilter(e.target.value as IntegrationCategory | 'all')}
                style={{
                  appearance: 'none', WebkitAppearance: 'none',
                  padding: '8px 32px 8px 12px', borderRadius: 6, fontSize: 12, fontWeight: 600,
                  border: `1px solid ${catFilter !== 'all' ? (CATEGORY_META[catFilter as IntegrationCategory]?.color || 'var(--border-md)') : 'var(--border-md)'}`,
                  background: catFilter !== 'all' ? `${CATEGORY_META[catFilter as IntegrationCategory]?.color || '#000'}10` : 'var(--bg-card)',
                  color: catFilter !== 'all' ? (CATEGORY_META[catFilter as IntegrationCategory]?.color || 'var(--text-2)') : 'var(--text-2)',
                  cursor: 'pointer', minWidth: 160,
                }}>
                <option value="all">All Categories ({INTEGRATION_CATALOG.length})</option>
                {(Object.entries(CATEGORY_META) as [IntegrationCategory, { label: string; color: string }][]).map(([key, meta]) => {
                  const count = INTEGRATION_CATALOG.filter(i => i.category === key).length;
                  return count > 0 ? (
                    <option key={key} value={key}>{meta.label} ({count})</option>
                  ) : null;
                })}
              </select>
              <Filter size={12} style={{ position: 'absolute', right: 10, top: '50%', transform: 'translateY(-50%)', pointerEvents: 'none', color: 'var(--text-4)' }} />
            </div>
          </div>

          {/* Category pills — quick filters */}
          <div style={{ display: 'flex', gap: 5, flexWrap: 'wrap', marginBottom: 16 }}>
            <button onClick={() => setCatFilter('all')} style={{
              padding: '4px 12px', borderRadius: 20, fontSize: 11, fontWeight: 600,
              border: '1px solid', cursor: 'pointer',
              background: catFilter === 'all' ? 'var(--primary-dim)' : 'transparent',
              borderColor: catFilter === 'all' ? 'var(--primary)' : 'var(--border)',
              color: catFilter === 'all' ? 'var(--primary-text)' : 'var(--text-3)',
            }}>All</button>
            {(Object.entries(CATEGORY_META) as [IntegrationCategory, { label: string; color: string }][]).map(([key, meta]) => {
              const count = INTEGRATION_CATALOG.filter(i => i.category === key).length;
              if (count === 0) return null;
              return (
                <button key={key} onClick={() => setCatFilter(catFilter === key ? 'all' : key)} style={{
                  padding: '4px 12px', borderRadius: 20, fontSize: 11, fontWeight: 600,
                  border: '1px solid', cursor: 'pointer',
                  background: catFilter === key ? `${meta.color}15` : 'transparent',
                  borderColor: catFilter === key ? meta.color : 'var(--border)',
                  color: catFilter === key ? meta.color : 'var(--text-3)',
                  transition: 'all 0.1s',
                }}>
                  {meta.label}
                  <span style={{ marginLeft: 4, opacity: 0.7 }}>{count}</span>
                </button>
              );
            })}
          </div>

          {/* Integration catalog grid */}
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(320px, 1fr))', gap: 10 }}>
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
                      <div style={{ display: 'flex', alignItems: 'center', gap: 6, flexWrap: 'wrap' }}>
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
                      {/* Capabilities */}
                      <div style={{ display: 'flex', gap: 4, flexWrap: 'wrap', marginTop: 8 }}>
                        {integration.capabilities.map(cap => {
                          const capMeta = CAPABILITY_META[cap];
                          return capMeta ? (
                            <span key={cap} style={{
                              fontSize: 9, padding: '1px 6px', borderRadius: 10,
                              background: `${capMeta.color}08`, border: `1px solid ${capMeta.color}20`,
                              color: capMeta.color, fontWeight: 500,
                            }}>{capMeta.label}</span>
                          ) : null;
                        })}
                      </div>
                      <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginTop: 6 }}>
                        <span style={{
                          fontSize: 10, color: 'var(--text-4)',
                          display: 'flex', alignItems: 'center', gap: 3,
                        }}>
                          <Shield size={9} /> {integration.authType.replace('_', ' ')}
                        </span>
                      </div>
                    </div>
                  </div>
                  <div style={{ display: 'flex', gap: 6, marginTop: 12, justifyContent: 'flex-end', alignItems: 'center' }}>
                    <a href={integration.docsUrl} target="_blank" rel="noopener noreferrer"
                      className="btn btn-ghost btn-sm"
                      style={{ fontSize: 11, color: 'var(--text-3)', textDecoration: 'none', display: 'flex', alignItems: 'center', gap: 4 }}
                      title={`${integration.name} developer docs — register app & get API keys`}>
                      <ExternalLink size={10} /> Docs
                    </a>
                    {connected ? (
                      <button className="btn btn-ghost btn-sm" style={{ fontSize: 11 }}
                        onClick={() => handleOpenConfigure(integration.id)}>
                        <Settings size={11} /> Configure
                      </button>
                    ) : (
                      <button className="btn btn-primary btn-sm" style={{ fontSize: 11 }}
                        onClick={() => {
                          setConnectingId(integration.id);
                          setCredClientId('');
                          setCredClientSecret('');
                          setCredSaved(false);
                          setShowCredSecrets(false);
                        }}>
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
        const callbackUrl = `${typeof window !== 'undefined' ? window.location.origin : ''}/api/oauth/${integration.id}/callback`;
        const isOAuth = integration.authType === 'oauth2';

        return (
          <div style={{
            position: 'fixed', inset: 0, background: 'rgba(0,0,0,0.4)',
            display: 'flex', alignItems: 'center', justifyContent: 'center', zIndex: 100,
          }}
            onClick={e => { if (e.target === e.currentTarget) { setConnectingId(null); setConnectConfig({}); } }}
          >
            <div style={{
              background: 'var(--bg-surface)', borderRadius: 10,
              padding: 0, width: 500, maxWidth: '92vw', maxHeight: '90vh', overflow: 'auto',
              boxShadow: '0 20px 60px rgba(0,0,0,0.2)',
            }}>
              {/* Modal header */}
              <div style={{
                display: 'flex', alignItems: 'center', gap: 12, padding: '20px 24px',
                borderBottom: '1px solid var(--border)',
              }}>
                <div style={{
                  width: 40, height: 40, borderRadius: 8, flexShrink: 0,
                  background: `${integration.color}12`, border: `1px solid ${integration.color}25`,
                  display: 'flex', alignItems: 'center', justifyContent: 'center',
                }}>
                  <ResolveIcon name={integration.icon} size={18} color={integration.color} />
                </div>
                <div style={{ flex: 1, minWidth: 0 }}>
                  <h3 style={{ fontSize: 16, fontWeight: 600, color: 'var(--text-1)', margin: 0 }}>
                    Connect {integration.name}
                  </h3>
                  <div style={{ fontSize: 12, color: 'var(--text-3)', marginTop: 2 }}>
                    {isOAuth ? 'OAuth 2.0 — Sign in with your account' : `Authentication: ${integration.authType.replace('_', ' ')}`}
                  </div>
                </div>
                <button onClick={() => { setConnectingId(null); setConnectConfig({}); }}
                  style={{ background: 'none', border: 'none', cursor: 'pointer', color: 'var(--text-4)', display: 'flex', padding: 4 }}>
                  <X size={16} />
                </button>
              </div>

              <div style={{ padding: '20px 24px' }}>

                {/* ── SETUP INSTRUCTIONS (OAuth) ── */}
                {isOAuth && (() => {
                  const hasSaved = !!savedCredentials[integration.id];
                  return (
                  <>
                    {/* Step 1: Developer Portal + Redirect URI */}
                    <div style={{
                      background: 'var(--bg)', border: '1px solid var(--border)',
                      borderRadius: 6, padding: '14px 16px', marginBottom: 16,
                    }}>
                      <div style={{ fontSize: 12, fontWeight: 700, color: 'var(--text-2)', marginBottom: 10, textTransform: 'uppercase', letterSpacing: '0.05em' }}>
                        Step 1 — Register your app
                      </div>
                      <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
                        <div style={{ display: 'flex', gap: 8, fontSize: 12, color: 'var(--text-3)', lineHeight: 1.5 }}>
                          <span style={{ fontWeight: 700, color: integration.color, minWidth: 14, flexShrink: 0 }}>a.</span>
                          <span>
                            Go to the{' '}
                            <a href={integration.docsUrl} target="_blank" rel="noopener noreferrer"
                              style={{ color: integration.color, fontWeight: 600, textDecoration: 'none' }}>
                              {integration.name} Developer Portal <ExternalLink size={9} style={{ display: 'inline', verticalAlign: 'middle' }} />
                            </a>
                            {' '}and create a new app.
                          </span>
                        </div>
                        <div style={{ display: 'flex', gap: 8, fontSize: 12, color: 'var(--text-3)', lineHeight: 1.5 }}>
                          <span style={{ fontWeight: 700, color: integration.color, minWidth: 14, flexShrink: 0 }}>b.</span>
                          <span>Add this <strong>Redirect URI</strong> to your app:</span>
                        </div>
                        <div style={{
                          display: 'flex', alignItems: 'center', gap: 6,
                          background: 'var(--bg-card)', border: '1px solid var(--border-md)',
                          borderRadius: 4, padding: '6px 10px', marginLeft: 22,
                        }}>
                          <code style={{ fontSize: 11, color: 'var(--text-1)', flex: 1, wordBreak: 'break-all', fontFamily: 'var(--font-mono, monospace)' }}>
                            {callbackUrl}
                          </code>
                          <button
                            onClick={() => { navigator.clipboard.writeText(callbackUrl); }}
                            className="btn btn-ghost btn-sm"
                            style={{ fontSize: 10, padding: '2px 8px', flexShrink: 0 }}
                            title="Copy to clipboard"
                          >
                            Copy
                          </button>
                        </div>
                      </div>
                    </div>

                    {/* Step 2: Enter credentials */}
                    <div style={{
                      background: 'var(--bg)', border: `1px solid ${hasSaved ? 'rgba(5,150,105,0.3)' : 'var(--border)'}`,
                      borderRadius: 6, padding: '14px 16px', marginBottom: 16,
                    }}>
                      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 12 }}>
                        <div style={{ fontSize: 12, fontWeight: 700, color: 'var(--text-2)', textTransform: 'uppercase', letterSpacing: '0.05em' }}>
                          Step 2 — Enter credentials
                        </div>
                        {hasSaved && (
                          <span style={{ fontSize: 10, fontWeight: 700, color: '#059669', display: 'flex', alignItems: 'center', gap: 3 }}>
                            <Check size={10} /> Saved ({savedCredentials[integration.id].clientIdMasked})
                          </span>
                        )}
                      </div>

                      {hasSaved && !credClientId && (
                        <div style={{
                          fontSize: 12, color: 'var(--text-3)', marginBottom: 12, padding: '8px 12px',
                          background: 'rgba(5,150,105,0.06)', borderRadius: 4, border: '1px solid rgba(5,150,105,0.15)',
                        }}>
                          Credentials are saved and encrypted. Enter new values below to update them.
                        </div>
                      )}

                      <div style={{ marginBottom: 10 }}>
                        <label style={{ fontSize: 12, fontWeight: 500, color: 'var(--text-2)', display: 'block', marginBottom: 4 }}>
                          Client ID
                        </label>
                        <input className="input" type="text"
                          placeholder={hasSaved ? `Current: ${savedCredentials[integration.id].clientIdMasked}` : 'Paste your Client ID...'}
                          style={{ width: '100%', fontSize: 12 }}
                          value={credClientId}
                          onChange={e => setCredClientId(e.target.value)} />
                      </div>
                      <div style={{ marginBottom: 12 }}>
                        <label style={{ fontSize: 12, fontWeight: 500, color: 'var(--text-2)', display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 4 }}>
                          Client Secret
                          <button onClick={() => setShowCredSecrets(!showCredSecrets)}
                            style={{ background: 'none', border: 'none', cursor: 'pointer', color: 'var(--text-4)', fontSize: 10, display: 'flex', alignItems: 'center', gap: 3 }}>
                            {showCredSecrets ? <><EyeOff size={10} /> Hide</> : <><Eye size={10} /> Show</>}
                          </button>
                        </label>
                        <input className="input" type={showCredSecrets ? 'text' : 'password'}
                          placeholder={hasSaved ? 'Enter new secret to update...' : 'Paste your Client Secret...'}
                          style={{ width: '100%', fontSize: 12 }}
                          value={credClientSecret}
                          onChange={e => setCredClientSecret(e.target.value)} />
                      </div>
                      <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                        <button
                          className="btn btn-secondary btn-sm"
                          style={{ fontSize: 11 }}
                          disabled={credSaving || (!credClientId.trim() || !credClientSecret.trim())}
                          onClick={() => handleSaveCredentials(integration.id)}
                        >
                          {credSaving ? 'Saving...' : credSaved ? <><Check size={11} /> Saved</> : <><Shield size={11} /> {hasSaved ? 'Update Keys' : 'Save Keys'}</>}
                        </button>
                        <span style={{ fontSize: 10, color: 'var(--text-4)' }}>
                          Encrypted with AES-256-GCM
                        </span>
                      </div>
                    </div>

                    {/* Step 3: Sign in */}
                    <div style={{
                      padding: '20px 16px', background: `${integration.color}06`,
                      border: `1px solid ${integration.color}20`, borderRadius: 6, marginBottom: 16, textAlign: 'center',
                    }}>
                      <div style={{ fontSize: 12, fontWeight: 700, color: 'var(--text-2)', marginBottom: 4, textTransform: 'uppercase', letterSpacing: '0.05em' }}>
                        Step 3 — Sign in
                      </div>
                      <div style={{ fontSize: 12, color: 'var(--text-3)', marginBottom: 14, lineHeight: 1.5 }}>
                        A new tab will open for you to sign in with {integration.name}.
                        Kortecx will never see your password.
                      </div>
                      <button
                        className="btn btn-primary btn-sm"
                        style={{ background: integration.color, borderColor: integration.color, fontSize: 13, padding: '9px 24px' }}
                        onClick={() => handleOAuthConnect(integration.id)}
                      >
                        <ExternalLink size={12} /> Continue with {integration.name}
                      </button>
                      <div style={{ fontSize: 10, color: 'var(--text-4)', marginTop: 10 }}>
                        Scopes: {integration.capabilities.join(' / ')}
                      </div>
                    </div>
                  </>
                  );
                })()}

                {/* ── SETUP INSTRUCTIONS (API Key / Bearer) ── */}
                {(integration.authType === 'api_key' || integration.authType === 'bearer') && (
                  <>
                    <div style={{
                      background: 'var(--bg)', border: '1px solid var(--border)',
                      borderRadius: 6, padding: '14px 16px', marginBottom: 16,
                    }}>
                      <div style={{ fontSize: 12, fontWeight: 700, color: 'var(--text-2)', marginBottom: 10, textTransform: 'uppercase', letterSpacing: '0.05em' }}>
                        How to get your {integration.authType === 'api_key' ? 'API Key' : 'Access Token'}
                      </div>
                      <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
                        <div style={{ display: 'flex', gap: 8, fontSize: 12, color: 'var(--text-3)', lineHeight: 1.5 }}>
                          <span style={{ fontWeight: 700, color: integration.color, minWidth: 18, flexShrink: 0 }}>1.</span>
                          <span>
                            Go to the{' '}
                            <a href={integration.docsUrl} target="_blank" rel="noopener noreferrer"
                              style={{ color: integration.color, fontWeight: 600, textDecoration: 'none' }}>
                              {integration.name} Developer Portal <ExternalLink size={9} style={{ display: 'inline', verticalAlign: 'middle' }} />
                            </a>
                          </span>
                        </div>
                        <div style={{ display: 'flex', gap: 8, fontSize: 12, color: 'var(--text-3)', lineHeight: 1.5 }}>
                          <span style={{ fontWeight: 700, color: integration.color, minWidth: 18, flexShrink: 0 }}>2.</span>
                          <span>Create or locate your {integration.authType === 'api_key' ? 'API key' : 'access token'} and paste it below.</span>
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
                  </>
                )}

                {/* ── SETUP INSTRUCTIONS (Basic auth) ── */}
                {integration.authType === 'basic' && (
                  <>
                    <div style={{
                      background: 'var(--bg)', border: '1px solid var(--border)',
                      borderRadius: 6, padding: '14px 16px', marginBottom: 16,
                    }}>
                      <div style={{ fontSize: 12, fontWeight: 700, color: 'var(--text-2)', marginBottom: 10, textTransform: 'uppercase', letterSpacing: '0.05em' }}>
                        Connection Details
                      </div>
                      <div style={{ fontSize: 12, color: 'var(--text-3)', lineHeight: 1.5 }}>
                        Enter your {integration.name} connection details below. See the{' '}
                        <a href={integration.docsUrl} target="_blank" rel="noopener noreferrer"
                          style={{ color: integration.color, fontWeight: 600, textDecoration: 'none' }}>
                          documentation <ExternalLink size={9} style={{ display: 'inline', verticalAlign: 'middle' }} />
                        </a>
                        {' '}for help finding your credentials.
                      </div>
                    </div>
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

                {/* Footer actions */}
                <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
                  <button className="btn btn-secondary btn-sm" onClick={() => { setConnectingId(null); setConnectConfig({}); }}>
                    Cancel
                  </button>
                  {!isOAuth && (
                    <button className="btn btn-primary btn-sm" onClick={() => handleConnect(integration.id)}>
                      <Check size={12} /> Connect
                    </button>
                  )}
                </div>
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
      {/* ── Configure Platform Modal ── */}
      {configuringId && (() => {
        const integration = INTEGRATION_CATALOG.find(i => i.id === configuringId);
        if (!integration) return null;

        const PERM_LABELS: Record<string, { label: string; description: string; color: string }> = {
          consume:  { label: 'Consume',  description: 'Read data, metrics, and analytics from the platform',    color: '#2563EB' },
          generate: { label: 'Generate', description: 'Use AI to create and optimize content for this platform', color: '#7C3AED' },
          publish:  { label: 'Publish',  description: 'Post, upload, or share content to this platform',         color: '#059669' },
          schedule: { label: 'Schedule', description: 'Schedule future posts and publications',                   color: '#D97706' },
          report:   { label: 'Report',   description: 'Generate analytics reports and performance summaries',    color: '#0EA5E9' },
          execute:  { label: 'Execute',  description: 'Perform actions like delete, edit, like, or follow',      color: '#DC2626' },
        };

        return (
          <div style={{
            position: 'fixed', inset: 0, background: 'rgba(0,0,0,0.4)',
            display: 'flex', alignItems: 'center', justifyContent: 'center', zIndex: 100,
          }}
            onClick={e => { if (e.target === e.currentTarget) setConfiguringId(null); }}
          >
            <div style={{
              background: 'var(--bg-surface)', borderRadius: 10,
              padding: 0, width: 520, maxWidth: '92vw', maxHeight: '90vh', overflow: 'auto',
              boxShadow: '0 20px 60px rgba(0,0,0,0.2)',
            }}>
              {/* Header */}
              <div style={{
                display: 'flex', alignItems: 'center', gap: 12, padding: '20px 24px',
                borderBottom: '1px solid var(--border)',
              }}>
                <div style={{
                  width: 40, height: 40, borderRadius: 8, flexShrink: 0,
                  background: `${integration.color}12`, border: `1px solid ${integration.color}25`,
                  display: 'flex', alignItems: 'center', justifyContent: 'center',
                }}>
                  <ResolveIcon name={integration.icon} size={18} color={integration.color} />
                </div>
                <div style={{ flex: 1 }}>
                  <h3 style={{ fontSize: 16, fontWeight: 600, color: 'var(--text-1)', margin: 0 }}>
                    Configure {integration.name}
                  </h3>
                  <div style={{ fontSize: 12, color: 'var(--text-3)', marginTop: 2 }}>
                    Manage permissions, tokens, and connection settings
                  </div>
                </div>
                <button onClick={() => setConfiguringId(null)}
                  style={{ background: 'none', border: 'none', cursor: 'pointer', color: 'var(--text-4)', display: 'flex', padding: 4 }}>
                  <X size={16} />
                </button>
              </div>

              <div style={{ padding: '20px 24px' }}>
                {configLoading ? (
                  <div style={{ textAlign: 'center', padding: '40px 0', color: 'var(--text-3)', fontSize: 13 }}>
                    Loading connection details...
                  </div>
                ) : !configData ? (
                  <div style={{ textAlign: 'center', padding: '40px 0', color: 'var(--text-3)', fontSize: 13 }}>
                    No connection found. Connect your account first.
                  </div>
                ) : (
                  <>
                    {/* Connection status */}
                    <div style={{
                      display: 'flex', alignItems: 'center', gap: 12, padding: '14px 16px',
                      background: 'var(--bg)', border: '1px solid var(--border)', borderRadius: 6, marginBottom: 16,
                    }}>
                      <div style={{
                        width: 36, height: 36, borderRadius: '50%', overflow: 'hidden',
                        background: `${integration.color}12`, border: `1px solid ${integration.color}25`,
                        display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0,
                      }}>
                        {configData.platformAvatar ? (
                          <img src={configData.platformAvatar} alt="" width={36} height={36} style={{ borderRadius: '50%', objectFit: 'cover' }} />
                        ) : (
                          <ResolveIcon name={integration.icon} size={16} color={integration.color} />
                        )}
                      </div>
                      <div style={{ flex: 1, minWidth: 0 }}>
                        <div style={{ fontSize: 13, fontWeight: 600, color: 'var(--text-1)' }}>
                          @{configData.platformUsername}
                        </div>
                        <div style={{ fontSize: 11, color: 'var(--text-3)', marginTop: 1 }}>
                          Connected {configData.connectedAt ? new Date(configData.connectedAt).toLocaleDateString() : ''}
                          {configData.lastUsedAt && <> · Last used {new Date(configData.lastUsedAt).toLocaleDateString()}</>}
                        </div>
                      </div>
                      <span style={{
                        fontSize: 10, fontWeight: 700, padding: '2px 8px', borderRadius: 4,
                        background: configData.isExpired ? 'rgba(217,119,6,0.1)' : configData.status === 'active' ? 'rgba(5,150,105,0.1)' : 'rgba(220,38,38,0.1)',
                        color: configData.isExpired ? '#D97706' : configData.status === 'active' ? '#059669' : '#DC2626',
                        textTransform: 'uppercase',
                      }}>{configData.isExpired ? 'EXPIRED' : configData.status}</span>
                    </div>

                    {/* Token management */}
                    <div style={{
                      background: 'var(--bg)', border: '1px solid var(--border)', borderRadius: 6,
                      padding: '14px 16px', marginBottom: 16,
                    }}>
                      <div style={{ fontSize: 12, fontWeight: 700, color: 'var(--text-2)', marginBottom: 10, textTransform: 'uppercase', letterSpacing: '0.05em' }}>
                        Token
                      </div>
                      <div style={{ display: 'flex', flexDirection: 'column', gap: 6, fontSize: 12 }}>
                        <div style={{ display: 'flex', justifyContent: 'space-between', color: 'var(--text-3)' }}>
                          <span>Status</span>
                          <span style={{ fontWeight: 600, color: configData.isExpired ? '#D97706' : '#059669' }}>
                            {configData.isExpired ? 'Expired' : 'Valid'}
                          </span>
                        </div>
                        {configData.tokenExpiresAt && (
                          <div style={{ display: 'flex', justifyContent: 'space-between', color: 'var(--text-3)' }}>
                            <span>Expires</span>
                            <span style={{ fontWeight: 500 }}>{new Date(configData.tokenExpiresAt).toLocaleString()}</span>
                          </div>
                        )}
                        {configData.lastRefreshedAt && (
                          <div style={{ display: 'flex', justifyContent: 'space-between', color: 'var(--text-3)' }}>
                            <span>Last refreshed</span>
                            <span style={{ fontWeight: 500 }}>{new Date(configData.lastRefreshedAt).toLocaleString()}</span>
                          </div>
                        )}
                        <div style={{ display: 'flex', justifyContent: 'space-between', color: 'var(--text-3)' }}>
                          <span>Refresh token</span>
                          <span style={{ fontWeight: 500 }}>{configData.hasRefreshToken ? 'Available' : 'Not available'}</span>
                        </div>
                      </div>
                      <div style={{ display: 'flex', gap: 8, marginTop: 12 }}>
                        <button
                          className="btn btn-secondary btn-sm"
                          style={{ fontSize: 11 }}
                          disabled={configRefreshing || !configData.hasRefreshToken}
                          onClick={() => handleRefreshToken(configuringId)}
                        >
                          {configRefreshing ? 'Refreshing...' : <><Activity size={11} /> Refresh Token</>}
                        </button>
                        <button
                          className="btn btn-ghost btn-sm"
                          style={{ fontSize: 11 }}
                          onClick={() => {
                            setConfiguringId(null);
                            handleOAuthConnect(configuringId);
                          }}
                        >
                          <ExternalLink size={11} /> Reconnect
                        </button>
                      </div>
                    </div>

                    {/* Permissions */}
                    <div style={{
                      background: 'var(--bg)', border: '1px solid var(--border)', borderRadius: 6,
                      padding: '14px 16px', marginBottom: 16,
                    }}>
                      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 12 }}>
                        <div style={{ fontSize: 12, fontWeight: 700, color: 'var(--text-2)', textTransform: 'uppercase', letterSpacing: '0.05em' }}>
                          Permissions
                        </div>
                        {permSaving && (
                          <span style={{ fontSize: 10, color: 'var(--text-4)' }}>Saving...</span>
                        )}
                      </div>
                      <div style={{ fontSize: 11, color: 'var(--text-4)', marginBottom: 12, lineHeight: 1.4 }}>
                        Control what Kortecx agents are allowed to do with this connection.
                        Disabled operations will be blocked even if the platform token has the scope.
                      </div>
                      <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
                        {Object.entries(PERM_LABELS).map(([key, meta]) => {
                          const platformCaps = integration.capabilities;
                          const isAvailable = platformCaps.includes(key as never);
                          const isEnabled = isAvailable && (configData.permissions[key] !== false);

                          return (
                            <div key={key} style={{
                              display: 'flex', alignItems: 'center', gap: 10, padding: '8px 10px',
                              background: isAvailable ? 'transparent' : 'var(--bg-card)',
                              borderRadius: 4, opacity: isAvailable ? 1 : 0.5,
                            }}>
                              {/* Toggle switch */}
                              <button
                                onClick={() => isAvailable && handleTogglePermission(configuringId, key, !isEnabled)}
                                disabled={!isAvailable}
                                style={{
                                  width: 36, height: 20, borderRadius: 10, padding: 2,
                                  border: 'none', cursor: isAvailable ? 'pointer' : 'not-allowed',
                                  background: isEnabled ? meta.color : 'var(--border-md)',
                                  transition: 'background 0.15s', flexShrink: 0,
                                  display: 'flex', alignItems: 'center',
                                  justifyContent: isEnabled ? 'flex-end' : 'flex-start',
                                }}
                              >
                                <div style={{
                                  width: 16, height: 16, borderRadius: '50%',
                                  background: '#fff', boxShadow: '0 1px 3px rgba(0,0,0,0.2)',
                                  transition: 'transform 0.15s',
                                }} />
                              </button>
                              <div style={{ flex: 1, minWidth: 0 }}>
                                <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                                  <span style={{ fontSize: 12, fontWeight: 600, color: isEnabled ? meta.color : 'var(--text-3)' }}>
                                    {meta.label}
                                  </span>
                                  {!isAvailable && (
                                    <span style={{ fontSize: 9, color: 'var(--text-4)', fontWeight: 500 }}>
                                      Not available for this platform
                                    </span>
                                  )}
                                </div>
                                <div style={{ fontSize: 11, color: 'var(--text-4)', marginTop: 1, lineHeight: 1.3 }}>
                                  {meta.description}
                                </div>
                              </div>
                            </div>
                          );
                        })}
                      </div>
                    </div>

                    {/* Scopes */}
                    {configData.scopes.length > 0 && (
                      <div style={{
                        background: 'var(--bg)', border: '1px solid var(--border)', borderRadius: 6,
                        padding: '14px 16px', marginBottom: 16,
                      }}>
                        <div style={{ fontSize: 12, fontWeight: 700, color: 'var(--text-2)', marginBottom: 10, textTransform: 'uppercase', letterSpacing: '0.05em' }}>
                          Granted Scopes
                        </div>
                        <div style={{ display: 'flex', flexWrap: 'wrap', gap: 4 }}>
                          {configData.scopes.map(scope => (
                            <span key={scope} style={{
                              fontSize: 10, padding: '2px 8px', borderRadius: 10,
                              background: `${integration.color}08`, border: `1px solid ${integration.color}20`,
                              color: integration.color, fontWeight: 500, fontFamily: 'var(--font-mono, monospace)',
                            }}>{scope}</span>
                          ))}
                        </div>
                      </div>
                    )}
                  </>
                )}

                {/* Footer */}
                <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
                  <button className="btn btn-secondary btn-sm" onClick={() => setConfiguringId(null)}>
                    Close
                  </button>
                </div>
              </div>
            </div>
          </div>
        );
      })()}
    </div>
  );
}
