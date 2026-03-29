'use client';

import { useState, useMemo, useEffect, useRef, Suspense } from 'react';
import { useSearchParams, useRouter } from 'next/navigation';
import Link from 'next/link';
import useSWR from 'swr';
import { motion, AnimatePresence } from 'framer-motion';

const fetcher = (url: string) => fetch(url).then(r => r.json());
import {
  Star, Search, Plus, Settings, TrendingUp,
  Loader2, Activity, Zap, Clock, BarChart2,
  ChevronDown, Play, Cpu, Tag, X,
  Trash2, CheckCircle2, AlertCircle,
  Copy, RotateCcw, Store, FileText,
  Network, LayoutGrid, Download,
} from 'lucide-react';
import { useExperts, usePrismGraph, useMarketplaceGraph } from '@/lib/hooks/useApi';
import { ROLE_META } from '@/lib/constants';
import type { Expert, ExpertRole } from '@/lib/types';
import ExpertEditDialog from './_components/ExpertEditDialog';
import PrismGraph from './_components/PrismGraph';
import PrismListView from './_components/PrismListView';
import { ImportButton, SharedImportButton } from '@/components/ImportExportButtons';
import SharedConfigImportDialog from '@/components/SharedConfigImportDialog';
import { exportEntity } from '@/lib/config-export';

/* ═══════════════════════════════════════════════════════
   Constants
   ═══════════════════════════════════════════════════════ */

import { fadeUp, stagger, hoverLift } from '@/lib/motion';

const SECTION_COLOR = '#D97706';

const staggerDefault = stagger();

/* ─── Status config ─────────────────────────────────── */
const STATUS_CONFIG: Record<string, { color: string; bg: string; label: string; pulse: boolean }> = {
  active:      { color: '#10b981', bg: '#10b98112', label: 'Active',      pulse: true  },
  idle:        { color: '#6b7280', bg: '#6b728012', label: 'Idle',        pulse: false },
  queued:      { color: '#f59e0b', bg: '#f59e0b12', label: 'Queued',      pulse: true  },
  running:     { color: '#3b82f6', bg: '#3b82f612', label: 'Running',     pulse: true  },
  completed:   { color: '#10b981', bg: '#10b98112', label: 'Completed',   pulse: false },
  failed:      { color: '#ef4444', bg: '#ef444412', label: 'Failed',      pulse: false },
  training:    { color: '#f59e0b', bg: '#f59e0b12', label: 'Training',    pulse: true  },
  finetuning:  { color: '#8b5cf6', bg: '#8b5cf612', label: 'Fine-tuning', pulse: true  },
  offline:     { color: '#ef4444', bg: '#ef444412', label: 'Offline',     pulse: false },
  error:       { color: '#ef4444', bg: '#ef444412', label: 'Error',       pulse: false },
};

/* ─── Provider config ───────────────────────────────── */
const PROVIDER_CONFIG: Record<string, { color: string; label: string }> = {
  anthropic: { color: '#D97757', label: 'Anthropic' },
  openai:    { color: '#74AA9C', label: 'OpenAI'    },
  google:    { color: '#4285f4', label: 'Google'    },
};

/* ─── Role emoji + color (for mine tab cards) ──────── */
const ROLE_EMOJI: Record<string, string> = {
  researcher: '🔬', analyst: '📊', writer: '✍️', coder: '💻',
  reviewer: '🔍', planner: '🗂', legal: '⚖️', financial: '💰',
  medical: '🩺', coordinator: '🔄', 'data-engineer': '🛠', creative: '🎨',
  translator: '🌐', custom: '⚙️',
};

const ROLE_COLOR: Record<string, string> = {
  researcher: '#8b5cf6', analyst: '#3b82f6', writer: '#f59e0b', coder: '#10b981',
  reviewer: '#06b6d4', planner: '#6366f1', legal: '#ef4444', financial: '#f97316',
  medical: '#ec4899', coordinator: '#8b5cf6', 'data-engineer': '#14b8a6',
  creative: '#a855f7', translator: '#06b6d4', custom: '#6b7280',
};

const STATUS_FILTERS = ['all', 'active', 'idle', 'queued', 'running', 'completed', 'failed', 'training', 'finetuning'] as const;
type StatusFilter = typeof STATUS_FILTERS[number];

const SORT_OPTIONS = [
  { value: 'rating', label: 'Rating'    },
  { value: 'runs',   label: 'Runs'      },
  { value: 'name',   label: 'Name'      },
  { value: 'cost',   label: 'Avg Cost'  },
] as const;
type SortOption = typeof SORT_OPTIONS[number]['value'];

type MarketplaceSortOption = 'rating' | 'runs' | 'name';

type TabKey = 'mine' | 'marketplace';

/* ─── Marketplace expert template type ─────────────── */
interface MarketplaceExpert {
  id: string;
  name: string;
  description: string;
  systemPrompt: string;
  role: ExpertRole;
  modelName: string;
  providerName: string;
  rating: number;
  totalRuns: number;
  tags: string[];
  specializations: string[];
  capabilities: string[];
}

/* ─── Marketplace experts data ─────────────────────── */
const MARKETPLACE_EXPERTS: MarketplaceExpert[] = [
  {
    id: 'mp-research-analyst-pro',
    name: 'Research Analyst Pro',
    description: 'Deep research agent for academic papers, literature reviews, and structured data analysis. Synthesizes findings from multiple sources into actionable insights with full citation tracking and evidence scoring.',
    systemPrompt: 'You are a meticulous research analyst. Gather evidence from multiple sources, cross-reference findings, assess credibility, and produce structured research reports with citations. Use data analysis to support every conclusion.',
    role: 'researcher',
    modelName: 'Claude Sonnet 4.6',
    providerName: 'Anthropic',
    rating: 4.8,
    totalRuns: 12400,
    tags: ['research', 'analysis', 'papers', 'synthesis'],
    specializations: ['Deep Research', 'Academic Papers', 'Data Analysis', 'Literature Review', 'Citation Tracking', 'Evidence Synthesis'],
    capabilities: ['reasoning', 'analysis', 'research', 'synthesis', 'writing'],
  },
  {
    id: 'mp-code-architect',
    name: 'Code Architect',
    description: 'System design and architecture expert for large-scale applications. Performs thorough code review, applies design patterns, generates technical documentation, and analyzes performance bottlenecks in distributed systems.',
    systemPrompt: 'You are a senior software architect. Review code for quality, suggest design patterns, document architecture decisions, and analyze system performance. Use data-driven reasoning to justify technical choices.',
    role: 'coder',
    modelName: 'Claude Opus 4.6',
    providerName: 'Anthropic',
    rating: 4.9,
    totalRuns: 15000,
    tags: ['architecture', 'code-review', 'design', 'performance'],
    specializations: ['System Design', 'Code Review', 'Architecture Patterns', 'Technical Docs', 'Performance Analysis', 'Data Analysis'],
    capabilities: ['coding', 'reasoning', 'analysis', 'review', 'writing'],
  },
  {
    id: 'mp-content-strategist',
    name: 'Content Strategist',
    description: 'Content marketing specialist with expertise in SEO optimization, brand voice development, and multi-channel content planning. Analyzes audience data and synthesizes trends into compelling editorial strategies.',
    systemPrompt: 'You are a content strategist. Analyze audience data, research trending topics, write compelling content, plan editorial calendars, and synthesize market research into brand-aligned messaging.',
    role: 'writer',
    modelName: 'GPT-4o',
    providerName: 'OpenAI',
    rating: 4.5,
    totalRuns: 8200,
    tags: ['content', 'seo', 'marketing', 'strategy'],
    specializations: ['Content Marketing', 'SEO Optimization', 'Brand Voice', 'Editorial Calendar', 'Audience Analysis', 'Data Analysis'],
    capabilities: ['writing', 'analysis', 'reasoning', 'research', 'synthesis'],
  },
  {
    id: 'mp-data-pipeline-engineer',
    name: 'Data Pipeline Engineer',
    description: 'ETL pipeline designer and optimizer specializing in data modeling, SQL optimization, and scalable data architectures. Reviews data quality, analyzes pipeline performance, and writes production-grade transformation code.',
    systemPrompt: 'You are a data engineer. Design ETL pipelines, optimize SQL queries, model data schemas, enforce data quality standards, and write clean transformation code. Analyze performance metrics to improve throughput.',
    role: 'data-engineer',
    modelName: 'Claude Sonnet 4.6',
    providerName: 'Anthropic',
    rating: 4.6,
    totalRuns: 6500,
    tags: ['etl', 'sql', 'data-modeling', 'pipelines'],
    specializations: ['ETL Pipelines', 'Data Modeling', 'SQL Optimization', 'Schema Design', 'Data Quality', 'Performance Analysis'],
    capabilities: ['coding', 'analysis', 'reasoning', 'data-processing', 'review'],
  },
  {
    id: 'mp-legal-compliance',
    name: 'Legal Compliance Advisor',
    description: 'Contract review and regulatory compliance expert. Analyzes legal documents, identifies risks, researches case precedents, and writes compliance reports with evidence-based policy recommendations.',
    systemPrompt: 'You are a legal compliance advisor. Review contracts, research regulations, assess risk exposure, analyze policy implications, and write detailed compliance reports. Support every recommendation with legal precedent.',
    role: 'legal',
    modelName: 'Claude Opus 4.6',
    providerName: 'Anthropic',
    rating: 4.7,
    totalRuns: 3800,
    tags: ['legal', 'compliance', 'contracts', 'risk'],
    specializations: ['Contract Review', 'Regulatory Compliance', 'Risk Assessment', 'Policy Analysis', 'Legal Research', 'Evidence Synthesis'],
    capabilities: ['reasoning', 'analysis', 'writing', 'research', 'review'],
  },
  {
    id: 'mp-financial-analyst',
    name: 'Financial Analyst',
    description: 'Financial modeling and market analysis specialist. Builds forecasts from structured data, evaluates risk scenarios, researches market trends, and generates comprehensive financial reports with synthesis of multiple data sources.',
    systemPrompt: 'You are a financial analyst. Build financial models, analyze market data, assess risk scenarios, research economic trends, and synthesize findings into actionable investment or budget recommendations.',
    role: 'financial',
    modelName: 'GPT-4o',
    providerName: 'OpenAI',
    rating: 4.4,
    totalRuns: 5100,
    tags: ['finance', 'modeling', 'risk', 'forecasting'],
    specializations: ['Financial Modeling', 'Market Analysis', 'Risk Assessment', 'Forecasting', 'Data Analysis', 'Evidence Synthesis'],
    capabilities: ['reasoning', 'analysis', 'research', 'synthesis', 'data-processing'],
  },
  {
    id: 'mp-creative-director',
    name: 'Creative Director',
    description: 'Branding and creative strategy expert. Develops visual concepts, writes creative briefs, researches design trends, and synthesizes audience insights into cohesive brand identity systems.',
    systemPrompt: 'You are a creative director. Research design trends, analyze audience preferences, develop brand concepts, write creative briefs, and synthesize market insights into compelling visual and verbal brand identities.',
    role: 'creative',
    modelName: 'Claude Sonnet 4.6',
    providerName: 'Anthropic',
    rating: 4.3,
    totalRuns: 4200,
    tags: ['branding', 'creative', 'design', 'strategy'],
    specializations: ['Branding', 'Visual Concepts', 'Creative Briefs', 'Brand Identity', 'Audience Analysis', 'Trend Research'],
    capabilities: ['writing', 'reasoning', 'analysis', 'research', 'synthesis'],
  },
  {
    id: 'mp-qa-reviewer',
    name: 'QA Reviewer',
    description: 'Code review and quality assurance specialist. Designs test plans, analyzes code coverage data, identifies bugs through systematic review, and writes detailed defect reports with reproduction steps.',
    systemPrompt: 'You are a QA reviewer. Review code for defects, design comprehensive test plans, analyze coverage metrics, enforce coding standards, and write detailed bug reports. Use data-driven quality assessment.',
    role: 'reviewer',
    modelName: 'Claude Haiku 4.5',
    providerName: 'Anthropic',
    rating: 4.2,
    totalRuns: 9800,
    tags: ['qa', 'testing', 'review', 'quality'],
    specializations: ['Code Review', 'Test Planning', 'Quality Assurance', 'Bug Detection', 'Standards Enforcement', 'Data Analysis'],
    capabilities: ['coding', 'analysis', 'review', 'reasoning', 'writing'],
  },
  {
    id: 'mp-project-coordinator',
    name: 'Project Coordinator',
    description: 'Project planning and task management expert. Decomposes complex goals into actionable plans, tracks progress with data analysis, writes stakeholder reports, and synthesizes team feedback into process improvements.',
    systemPrompt: 'You are a project coordinator. Decompose goals into tasks, track progress metrics, analyze team velocity data, write status reports for stakeholders, and synthesize feedback into actionable improvements.',
    role: 'coordinator',
    modelName: 'Gemini 2.5 Pro',
    providerName: 'Google',
    rating: 4.1,
    totalRuns: 3200,
    tags: ['project', 'planning', 'management', 'coordination'],
    specializations: ['Project Planning', 'Task Decomposition', 'Status Tracking', 'Stakeholder Reports', 'Data Analysis', 'Process Synthesis'],
    capabilities: ['reasoning', 'writing', 'analysis', 'planning', 'synthesis'],
  },
  {
    id: 'mp-medical-research',
    name: 'Medical Research Analyst',
    description: 'Clinical data and medical literature specialist. Reviews published studies, analyzes patient data patterns, researches treatment outcomes, and synthesizes evidence into clinical decision support recommendations.',
    systemPrompt: 'You are a medical research analyst. Review clinical studies, analyze health data, research treatment outcomes, assess evidence quality, and synthesize findings into evidence-based clinical recommendations.',
    role: 'medical',
    modelName: 'Claude Opus 4.6',
    providerName: 'Anthropic',
    rating: 4.8,
    totalRuns: 2100,
    tags: ['medical', 'clinical', 'health', 'evidence-based'],
    specializations: ['Clinical Data', 'Medical Literature', 'Health Informatics', 'Evidence Synthesis', 'Data Analysis', 'Treatment Research'],
    capabilities: ['reasoning', 'analysis', 'research', 'synthesis', 'data-processing'],
  },
  {
    id: 'mp-strategy-planner',
    name: 'Strategy Planner',
    description: 'Business strategy and roadmap planning specialist. Researches competitive landscapes, analyzes market data, develops OKR frameworks, and synthesizes insights into strategic recommendations with evidence-backed priorities.',
    systemPrompt: 'You are a strategy planner. Research market landscapes, analyze competitive data, develop OKR frameworks, plan strategic roadmaps, and synthesize multi-source insights into prioritized recommendations.',
    role: 'planner',
    modelName: 'GPT-4o',
    providerName: 'OpenAI',
    rating: 4.3,
    totalRuns: 4700,
    tags: ['strategy', 'roadmap', 'okr', 'planning'],
    specializations: ['Business Strategy', 'Roadmap Planning', 'OKR Frameworks', 'Competitive Analysis', 'Market Research', 'Data Analysis'],
    capabilities: ['reasoning', 'writing', 'analysis', 'research', 'planning', 'synthesis'],
  },
  {
    id: 'mp-translation-specialist',
    name: 'Translation Specialist',
    description: 'Multilingual translation and localization expert. Handles cultural adaptation, researches regional conventions, analyzes terminology consistency, and writes localized content that preserves meaning across languages.',
    systemPrompt: 'You are a translation specialist. Translate content accurately, research cultural contexts, analyze terminology consistency, adapt messaging for regional audiences, and write localized copy that preserves intent.',
    role: 'translator',
    modelName: 'Gemini 2.5 Pro',
    providerName: 'Google',
    rating: 4.5,
    totalRuns: 7600,
    tags: ['translation', 'localization', 'multilingual', 'adaptation'],
    specializations: ['Multilingual Translation', 'Localization', 'Cultural Adaptation', 'Terminology Management', 'Content Analysis', 'Regional Research'],
    capabilities: ['writing', 'reasoning', 'analysis', 'research', 'communication'],
  },
];

/* ─── Helpers ──────────────────────────────────────── */
function fmt(n: number) {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(0)}k`;
  return String(n);
}

function pct(n: number) {
  return `${(n * 100).toFixed(1)}%`;
}

/* ═══════════════════════════════════════════════════════
   Skeleton Card
   ═══════════════════════════════════════════════════════ */
function SkeletonCard() {
  return (
    <div style={{
      background: 'var(--bg-surface)',
      border: '1px solid var(--border)',
      borderRadius: 12, padding: 20,
      display: 'flex', flexDirection: 'column', gap: 14,
    }}>
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start' }}>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 8, flex: 1 }}>
          <div className="skeleton" style={{ height: 14, width: '55%', borderRadius: 6 }} />
          <div style={{ display: 'flex', gap: 6 }}>
            <div className="skeleton" style={{ height: 20, width: 58, borderRadius: 99 }} />
            <div className="skeleton" style={{ height: 20, width: 68, borderRadius: 99 }} />
          </div>
        </div>
        <div className="skeleton" style={{ width: 36, height: 36, borderRadius: 8 }} />
      </div>
      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 1fr', gap: 8 }}>
        {[0, 1, 2].map(i => (
          <div key={i} className="skeleton" style={{ height: 56, borderRadius: 8 }} />
        ))}
      </div>
      <div className="skeleton" style={{ height: 22, width: '70%', borderRadius: 99 }} />
      <div style={{ display: 'flex', gap: 7 }}>
        {[0, 1, 2].map(i => (
          <div key={i} className="skeleton" style={{ height: 30, flex: 1, borderRadius: 7 }} />
        ))}
      </div>
    </div>
  );
}

/* ═══════════════════════════════════════════════════════
   Stats Modal
   ═══════════════════════════════════════════════════════ */
function StatsModal({
  expert,
  onClose,
}: {
  expert: Record<string, unknown>;
  onClose: () => void;
}) {
  const role      = (expert.role as string) ?? 'custom';
  const roleColor = ROLE_COLOR[role] ?? '#6b7280';
  const roleEmoji = ROLE_EMOJI[role] ?? '⚙️';
  const statusKey = (expert.status as string) ?? 'idle';
  const statusCfg = STATUS_CONFIG[statusKey] ?? STATUS_CONFIG.idle;

  const stats       = (expert.stats as Record<string, number>) ?? {};
  const totalRuns   = (expert.totalRuns as number) ?? stats.totalRuns ?? 0;
  const successRate = (expert.successRate as number) ?? stats.successRate ?? 0;
  const avgCost     = stats.avgCostPerRun ?? 0;
  const avgLatency  = (expert.avgLatencyMs as number) ?? stats.avgLatencyMs ?? 0;
  const rating      = stats.rating ?? 0;
  const avgTokens   = stats.avgTokensPerRun ?? 0;

  const failureRate = successRate > 0 ? 1 - successRate : 0;
  const totalCost   = avgCost * totalRuns;
  const totalTokens = avgTokens * totalRuns;

  const statBlocks: Array<{ label: string; value: string; color: string; sub: string }> = [
    { label: 'Total Runs',     value: fmt(totalRuns),                                    color: SECTION_COLOR, sub: 'all time' },
    { label: 'Success Rate',   value: successRate > 0 ? pct(successRate) : '—',          color: '#10b981',     sub: `${Math.round(successRate * totalRuns)} succeeded` },
    { label: 'Failure Rate',   value: failureRate > 0 ? pct(failureRate) : '—',          color: '#ef4444',     sub: `${Math.round(failureRate * totalRuns)} failed` },
    { label: 'Avg Latency',    value: avgLatency > 0 ? `${avgLatency.toLocaleString()} ms` : '—', color: '#06b6d4', sub: 'per run' },
    { label: 'Avg Cost',       value: avgCost > 0 ? `$${avgCost.toFixed(4)}` : '—',     color: '#f59e0b',     sub: 'per run' },
    { label: 'Total Cost',     value: totalCost > 0 ? `$${totalCost.toFixed(2)}` : '—', color: '#f97316',     sub: 'all time' },
    { label: 'Avg Tokens',     value: avgTokens > 0 ? fmt(avgTokens) : '—',             color: '#8b5cf6',     sub: 'per run' },
    { label: 'Total Tokens',   value: totalTokens > 0 ? fmt(totalTokens) : '—',         color: '#6366f1',     sub: 'all time' },
    { label: 'Rating',         value: rating > 0 ? `${rating.toFixed(1)} / 5` : '—',    color: '#f59e0b',     sub: rating > 0 ? `${(rating / 5 * 100).toFixed(0)}% score` : 'not rated' },
  ];

  return (
    <div style={{
      position: 'fixed', inset: 0, background: 'rgba(7,7,26,0.85)',
      zIndex: 200, display: 'flex', alignItems: 'flex-start', justifyContent: 'center',
      paddingTop: 60, overflowY: 'auto',
    }}>
      <motion.div
        initial={{ opacity: 0, y: 16, scale: 0.97 }}
        animate={{ opacity: 1, y: 0, scale: 1 }}
        exit={{ opacity: 0, y: 8 }}
        transition={{ duration: 0.2 }}
        style={{
          width: 580, maxWidth: '92vw',
          background: 'var(--bg-surface)', border: '1px solid var(--border-md)',
          borderRadius: 12, overflow: 'hidden',
          boxShadow: '0 24px 80px rgba(0,0,0,0.2)',
          marginBottom: 40,
        }}
      >
        {/* Header */}
        <div style={{
          padding: '18px 22px', borderBottom: '1px solid var(--border)',
          display: 'flex', alignItems: 'center', justifyContent: 'space-between',
        }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
            <BarChart2 size={16} color={SECTION_COLOR} />
            <span style={{ fontSize: 15, fontWeight: 700, color: 'var(--text-1)' }}>PRISM Statistics</span>
          </div>
          <button onClick={onClose} style={{
            background: 'none', border: 'none', cursor: 'pointer',
            color: 'var(--text-3)', display: 'flex', padding: 4,
          }}>
            <X size={16} />
          </button>
        </div>

        {/* Expert identity */}
        <div style={{
          padding: '16px 22px', borderBottom: '1px solid var(--border)',
          display: 'flex', alignItems: 'center', gap: 14,
        }}>
          <div style={{
            width: 48, height: 48, borderRadius: 10, flexShrink: 0,
            background: `${roleColor}12`, border: `1.5px solid ${roleColor}25`,
            display: 'flex', alignItems: 'center', justifyContent: 'center',
            fontSize: 22,
          }}>
            {roleEmoji}
          </div>
          <div style={{ flex: 1 }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
              <span style={{ fontSize: 16, fontWeight: 700, color: 'var(--text-1)' }}>
                {expert.name as string}
              </span>
              <span style={{
                padding: '2px 8px', borderRadius: 99, fontSize: 10, fontWeight: 700,
                background: statusCfg.bg, color: statusCfg.color,
                border: `1px solid ${statusCfg.color}28`,
              }}>{statusCfg.label}</span>
            </div>
            <div style={{ fontSize: 12, color: 'var(--text-3)', marginTop: 3 }}>
              <span style={{ textTransform: 'capitalize' }}>{role}</span>
              <span style={{ color: 'var(--text-4)' }}> · </span>
              {(expert.modelName ?? expert.modelId) as string}
              <span style={{ color: 'var(--text-4)' }}> · </span>
              {(expert.providerName ?? expert.providerId) as string}
            </div>
          </div>
          {rating > 0 && (
            <div style={{ display: 'flex', alignItems: 'center', gap: 4, color: '#f59e0b' }}>
              <Star size={16} fill="#f59e0b" strokeWidth={0} />
              <span style={{ fontSize: 18, fontWeight: 800 }}>{rating.toFixed(1)}</span>
            </div>
          )}
        </div>

        {/* Stats grid */}
        <div style={{ padding: '20px 22px' }}>
          <div style={{
            display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)',
            gap: 10,
          }}>
            {statBlocks.map(block => (
              <div key={block.label} style={{
                padding: '14px 16px', borderRadius: 10,
                background: `${block.color}06`, border: `1px solid ${block.color}15`,
              }}>
                <div style={{ fontSize: 24, fontWeight: 800, color: block.color, lineHeight: 1 }}>
                  {block.value}
                </div>
                <div style={{ fontSize: 11, fontWeight: 600, color: 'var(--text-2)', marginTop: 6 }}>
                  {block.label}
                </div>
                <div style={{ fontSize: 10, color: 'var(--text-4)', marginTop: 2 }}>
                  {block.sub}
                </div>
              </div>
            ))}
          </div>

          {/* Success rate bar */}
          {totalRuns > 0 && (
            <div style={{ marginTop: 20 }}>
              <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: 6 }}>
                <span style={{ fontSize: 11, fontWeight: 600, color: 'var(--text-2)' }}>Success Rate Distribution</span>
                <span style={{ fontSize: 11, fontWeight: 700, color: '#10b981' }}>{pct(successRate)}</span>
              </div>
              <div style={{
                height: 8, borderRadius: 99, background: 'var(--bg-elevated)',
                border: '1px solid var(--border)', overflow: 'hidden',
                display: 'flex',
              }}>
                <div style={{
                  width: `${successRate * 100}%`, height: '100%',
                  background: 'linear-gradient(90deg, #10b981, #059669)',
                  borderRadius: 99,
                  transition: 'width 0.4s ease',
                }} />
                {failureRate > 0 && (
                  <div style={{
                    width: `${failureRate * 100}%`, height: '100%',
                    background: 'linear-gradient(90deg, #ef4444, #dc2626)',
                  }} />
                )}
              </div>
              <div style={{ display: 'flex', justifyContent: 'space-between', marginTop: 4 }}>
                <span style={{ fontSize: 10, color: '#10b981', display: 'flex', alignItems: 'center', gap: 3 }}>
                  <CheckCircle2 size={9} /> {Math.round(successRate * totalRuns)} succeeded
                </span>
                {failureRate > 0 && (
                  <span style={{ fontSize: 10, color: '#ef4444', display: 'flex', alignItems: 'center', gap: 3 }}>
                    <AlertCircle size={9} /> {Math.round(failureRate * totalRuns)} failed
                  </span>
                )}
              </div>
            </div>
          )}

          {totalRuns === 0 && (
            <div style={{
              marginTop: 20, padding: '24px 16px', textAlign: 'center',
              border: '1px dashed var(--border-md)', borderRadius: 8,
            }}>
              <BarChart2 size={20} color="var(--text-4)" style={{ margin: '0 auto 8px' }} />
              <div style={{ fontSize: 13, color: 'var(--text-3)', fontWeight: 500 }}>No run data yet</div>
              <div style={{ fontSize: 11, color: 'var(--text-4)', marginTop: 4 }}>
                Run this PRISM in a workflow to start collecting statistics.
              </div>
            </div>
          )}
        </div>

        {/* Footer */}
        <div style={{
          padding: '14px 22px', borderTop: '1px solid var(--border)',
          display: 'flex', justifyContent: 'flex-end',
        }}>
          <button onClick={onClose} style={{
            padding: '8px 18px', borderRadius: 7, fontSize: 12, fontWeight: 600,
            border: '1px solid var(--border-md)', background: 'transparent',
            color: 'var(--text-2)', cursor: 'pointer',
          }}>Close</button>
        </div>
      </motion.div>
    </div>
  );
}

/* ═══════════════════════════════════════════════════════
   My Expert Card
   ═══════════════════════════════════════════════════════ */
function MyExpertCard({
  expert,
  onConfigure,
  onViewStats,
  onDelete,
  onExport,
  onRun,
  highlighted,
  cardRef,
  runStatus: _runStatus,
}: {
  expert: Record<string, unknown>;
  onConfigure: () => void;
  onViewStats: () => void;
  onDelete: () => void;
  onExport: () => void;
  onRun: () => void;
  highlighted: boolean;
  cardRef?: React.Ref<HTMLDivElement>;
  runStatus?: 'running' | 'success' | 'error';
}) {
  const role        = (expert.role as string) ?? 'custom';
  const roleColor   = ROLE_COLOR[role] ?? '#6b7280';
  const roleEmoji   = ROLE_EMOJI[role] ?? '⚙️';
  const statusKey   = (expert.status as string) ?? 'idle';
  const status      = STATUS_CONFIG[statusKey] ?? STATUS_CONFIG.idle;
  const providerId  = (expert.providerId as string) ?? '';
  const provider    = PROVIDER_CONFIG[providerId.toLowerCase()] ?? { color: '#6b7280', label: expert.providerName as string ?? 'Unknown' };

  const stats       = (expert.stats as Record<string, number>) ?? {};
  const totalRuns   = (expert.totalRuns as number) ?? stats.totalRuns ?? 0;
  const successRate = (expert.successRate as number) ?? stats.successRate ?? 0;
  const _avgCost    = stats.avgCostPerRun ?? 0;
  const avgLatency  = (expert.avgLatencyMs as number) ?? stats.avgLatencyMs ?? 0;
  const avgTokens   = stats.avgTokensPerRun ?? 0;
  const cpuUsage    = (expert.metadata as Record<string, unknown> | undefined)?.cpuUsage as number | undefined;
  const _rating     = stats.rating ?? 0;
  const tags        = ((expert.tags as string[]) ?? []).slice(0, 3);
  const isFinetuned = (expert.isFinetuned as boolean) ?? false;

  return (
    <motion.div
      ref={cardRef}
      variants={fadeUp}
      whileHover={{ y: -3, boxShadow: '0 10px 32px rgba(13,13,13,0.09)' }}
      transition={{ type: 'spring', stiffness: 380, damping: 28 }}
      onClick={onConfigure}
      style={{
        background: 'var(--bg-surface)',
        border: highlighted ? `2px solid ${SECTION_COLOR}` : '1px solid var(--border)',
        borderRadius: 12, padding: 20,
        display: 'flex', flexDirection: 'column', gap: 14,
        position: 'relative', overflow: 'hidden', cursor: 'pointer',
        animation: highlighted ? 'highlight-pulse 0.6s ease-in-out 3' : undefined,
      }}
    >
      {/* Top accent stripe */}
      <div style={{
        position: 'absolute', top: 0, left: 0, right: 0, height: 2,
        background: `linear-gradient(90deg, ${roleColor}, ${roleColor}50)`,
        borderRadius: '12px 12px 0 0',
      }} />

      {/* Header: name + role icon */}
      <div style={{ display: 'flex', alignItems: 'flex-start', justifyContent: 'space-between', marginTop: 4 }}>
        <div style={{ flex: 1, minWidth: 0 }}>
          {/* Name + version */}
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
            <span style={{ fontSize: 15, fontWeight: 700, color: 'var(--text-1)', lineHeight: 1.2 }}>
              {expert.name as string}
            </span>
            <span style={{
              padding: '1px 7px', borderRadius: 99, fontSize: 10, fontWeight: 700,
              background: 'var(--bg-elevated)', color: 'var(--text-3)',
              border: '1px solid var(--border-md)',
            }}>
              v{expert.version as string}
            </span>
          </div>

          {/* Status + badges row */}
          <div style={{ display: 'flex', alignItems: 'center', gap: 6, marginTop: 7, flexWrap: 'wrap' }}>
            {/* Status dot + label */}
            <div style={{
              display: 'flex', alignItems: 'center', gap: 5,
              padding: '3px 8px', borderRadius: 99,
              background: status.bg, border: `1px solid ${status.color}28`,
            }}>
              <div style={{ position: 'relative', width: 6, height: 6 }}>
                {status.pulse && (
                  <div className="dot-pulse" style={{
                    position: 'absolute', inset: -3,
                    borderRadius: '50%', background: `${status.color}30`,
                  }} />
                )}
                <div style={{
                  width: 6, height: 6, borderRadius: '50%',
                  background: status.color, position: 'relative', zIndex: 1,
                }} />
              </div>
              <span style={{ fontSize: 10, fontWeight: 700, color: status.color }}>
                {status.label}
              </span>
            </div>

            {/* Provider badge */}
            <span style={{
              padding: '3px 8px', borderRadius: 99, fontSize: 10, fontWeight: 700,
              background: `${provider.color}12`, color: provider.color,
              border: `1px solid ${provider.color}28`,
            }}>
              {provider.label}
            </span>

            {/* Fine-tuned badge */}
            {isFinetuned && (
              <span style={{
                padding: '3px 8px', borderRadius: 99, fontSize: 10, fontWeight: 700,
                background: `${SECTION_COLOR}12`, color: SECTION_COLOR,
                border: `1px solid ${SECTION_COLOR}30`,
              }}>
                ✦ Fine-tuned
              </span>
            )}
          </div>
        </div>

        {/* Role emoji icon */}
        <div style={{
          width: 40, height: 40, borderRadius: 9, flexShrink: 0, marginLeft: 8,
          background: `${roleColor}12`, border: `1.5px solid ${roleColor}25`,
          display: 'flex', alignItems: 'center', justifyContent: 'center',
          fontSize: 18,
        }}>
          {roleEmoji}
        </div>
      </div>

      {/* Role name */}
      <div style={{
        display: 'flex', alignItems: 'center', gap: 6,
        fontSize: 11, color: 'var(--text-3)', fontWeight: 500,
        marginTop: -6,
      }}>
        <Cpu size={10} color="var(--text-4)" />
        <span style={{ textTransform: 'capitalize' }}>{role}</span>
        <span style={{ color: 'var(--text-4)' }}>·</span>
        <span style={{ color: 'var(--text-4)' }}>{(expert.modelName ?? expert.modelId) as string}</span>
      </div>

      {/* Stats grid */}
      <div style={{
        display: 'grid', gridTemplateColumns: '1fr 1fr 1fr',
        gap: 6, paddingTop: 10, borderTop: '1px solid var(--border)',
      }}>
        <div style={{ textAlign: 'center', padding: '7px 4px', borderRadius: 6, background: `${SECTION_COLOR}06`, border: `1px solid ${SECTION_COLOR}12` }}>
          <div style={{ fontSize: 16, fontWeight: 800, color: SECTION_COLOR, lineHeight: 1 }}>{fmt(totalRuns)}</div>
          <div style={{ fontSize: 8, color: 'var(--text-4)', marginTop: 2, fontWeight: 500 }}>Runs</div>
        </div>
        <div style={{ textAlign: 'center', padding: '7px 4px', borderRadius: 6, background: '#10b98106', border: '1px solid #10b98112' }}>
          <div style={{ fontSize: 16, fontWeight: 800, color: '#10b981', lineHeight: 1 }}>{successRate > 0 ? pct(successRate) : '—'}</div>
          <div style={{ fontSize: 8, color: 'var(--text-4)', marginTop: 2, fontWeight: 500 }}>Success</div>
        </div>
        <div style={{ textAlign: 'center', padding: '7px 4px', borderRadius: 6, background: '#2563EB06', border: '1px solid #2563EB12' }}>
          <div style={{ fontSize: 16, fontWeight: 800, color: '#2563EB', lineHeight: 1 }}>{avgLatency > 0 ? `${(avgLatency / 1000).toFixed(1)}s` : '—'}</div>
          <div style={{ fontSize: 8, color: 'var(--text-4)', marginTop: 2, fontWeight: 500 }}>Avg Time</div>
        </div>
        <div style={{ textAlign: 'center', padding: '7px 4px', borderRadius: 6, background: '#8b5cf606', border: '1px solid #8b5cf612' }}>
          <div style={{ fontSize: 16, fontWeight: 800, color: '#8b5cf6', lineHeight: 1 }}>{avgLatency > 0 ? `${(avgLatency / 1000).toFixed(1)}s` : '—'}</div>
          <div style={{ fontSize: 8, color: 'var(--text-4)', marginTop: 2, fontWeight: 500 }}>Run Time</div>
        </div>
        <div style={{ textAlign: 'center', padding: '7px 4px', borderRadius: 6, background: '#f59e0b06', border: '1px solid #f59e0b12' }}>
          <div style={{ fontSize: 16, fontWeight: 800, color: '#f59e0b', lineHeight: 1 }}>{cpuUsage != null ? `${cpuUsage}%` : '—'}</div>
          <div style={{ fontSize: 8, color: 'var(--text-4)', marginTop: 2, fontWeight: 500 }}>CPU Usage</div>
        </div>
        <div style={{ textAlign: 'center', padding: '7px 4px', borderRadius: 6, background: '#ec489906', border: '1px solid #ec489912' }}>
          <div style={{ fontSize: 16, fontWeight: 800, color: '#ec4899', lineHeight: 1 }}>{avgTokens > 0 ? fmt(avgTokens) : '—'}</div>
          <div style={{ fontSize: 8, color: 'var(--text-4)', marginTop: 2, fontWeight: 500 }}>Tokens Used</div>
        </div>
      </div>

      {/* Tags */}
      {tags.length > 0 && (
        <div style={{ display: 'flex', alignItems: 'center', gap: 6, flexWrap: 'wrap' }}>
          <Tag size={10} color="var(--text-4)" />
          {tags.map(tag => (
            <span key={tag} style={{
              padding: '2px 8px', borderRadius: 99, fontSize: 10, fontWeight: 500,
              background: 'var(--bg-elevated)', color: 'var(--text-3)',
              border: '1px solid var(--border)',
            }}>
              {tag}
            </span>
          ))}
        </div>
      )}

      {/* Artifact location */}
      {totalRuns > 0 && (
        <div style={{ display: 'flex', alignItems: 'center', gap: 6, fontSize: 10, color: 'var(--text-4)', padding: '4px 0' }}>
          <FileText size={9} />
          <span className="mono" style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
            experts/{(expert.modelSource as string) === 'local' ? 'local' : 'marketplace'}/{(expert.name as string).toLowerCase().replace(/[^a-z0-9]+/g, '-')}/
          </span>
        </div>
      )}

      {/* Action buttons */}
      <div style={{
        display: 'flex', gap: 5, paddingTop: 4,
        borderTop: '1px solid var(--border)', marginTop: 'auto',
      }}>
        <button onClick={e => { e.stopPropagation(); onRun(); }} style={{
          flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 5,
          padding: '7px 8px', borderRadius: 7, cursor: 'pointer',
          border: `1.5px solid ${SECTION_COLOR}50`,
          background: `${SECTION_COLOR}12`,
          color: SECTION_COLOR, fontSize: 11, fontWeight: 700,
          transition: 'all 0.15s',
        }}>
          <Play size={10} fill={SECTION_COLOR} strokeWidth={0} />
          Run
        </button>
        <button onClick={e => { e.stopPropagation(); onConfigure(); }} style={{
          flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 5,
          padding: '7px 8px', borderRadius: 7, cursor: 'pointer',
          border: '1px solid var(--border-md)',
          background: 'transparent',
          color: 'var(--text-3)', fontSize: 11, fontWeight: 500,
          transition: 'all 0.15s',
        }}>
          <Settings size={10} />
          Configure
        </button>
        <button onClick={e => { e.stopPropagation(); onViewStats(); }} style={{
          flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 5,
          padding: '7px 8px', borderRadius: 7, cursor: 'pointer',
          border: '1px solid var(--border-md)',
          background: 'transparent',
          color: 'var(--text-3)', fontSize: 11, fontWeight: 500,
          transition: 'all 0.15s',
        }}>
          <BarChart2 size={10} />
          Stats
        </button>
        <button onClick={e => { e.stopPropagation(); onExport(); }} title="Export PRISM" style={{
          display: 'flex', alignItems: 'center', justifyContent: 'center',
          padding: '7px 8px', borderRadius: 7, cursor: 'pointer',
          border: '1px solid var(--border-md)',
          background: 'transparent',
          color: 'var(--text-4)', fontSize: 11,
          transition: 'all 0.15s',
        }}>
          <Download size={11} />
        </button>
        <button onClick={e => { e.stopPropagation(); onDelete(); }} title="Delete PRISM" style={{
          display: 'flex', alignItems: 'center', justifyContent: 'center',
          padding: '7px 8px', borderRadius: 7, cursor: 'pointer',
          border: '1px solid rgba(220,38,38,0.2)',
          background: 'transparent',
          color: 'var(--text-4)', fontSize: 11,
          transition: 'all 0.15s',
        }}
          onMouseEnter={e => { e.currentTarget.style.color = '#DC2626'; e.currentTarget.style.background = 'rgba(220,38,38,0.06)'; e.currentTarget.style.borderColor = '#DC2626'; }}
          onMouseLeave={e => { e.currentTarget.style.color = 'var(--text-4)'; e.currentTarget.style.background = 'transparent'; e.currentTarget.style.borderColor = 'rgba(220,38,38,0.2)'; }}
        >
          <Trash2 size={11} />
        </button>
      </div>
    </motion.div>
  );
}

/* ═══════════════════════════════════════════════════════
   Marketplace Expert Card
   ═══════════════════════════════════════════════════════ */
function MarketplaceCard({ expert }: { expert: MarketplaceExpert }) {
  const roleMeta = ROLE_META[expert.role] ?? ROLE_META.custom;
  const roleColor = ROLE_COLOR[expert.role] ?? '#6b7280';
  const roleEmoji = ROLE_EMOJI[expert.role] ?? '⚙️';
  const providerColor = PROVIDER_CONFIG[expert.providerName.toLowerCase()]?.color ?? '#6b7280';

  return (
    <motion.div
      variants={fadeUp}
      {...hoverLift}
      style={{
        background: 'var(--bg-surface)',
        border: '1px solid var(--border)',
        borderRadius: 12, padding: 20,
        display: 'flex', flexDirection: 'column', gap: 12,
        position: 'relative', overflow: 'hidden',
      }}
    >
      {/* Top accent stripe */}
      <div style={{
        position: 'absolute', top: 0, left: 0, right: 0, height: 2,
        background: `linear-gradient(90deg, ${roleColor}, ${roleColor}50)`,
        borderRadius: '12px 12px 0 0',
      }} />

      {/* Header */}
      <div style={{ display: 'flex', alignItems: 'flex-start', gap: 12, marginTop: 4 }}>
        <div className="expert-avatar" style={{
          width: 44, height: 44, borderRadius: 10, flexShrink: 0,
          background: `${roleColor}12`, border: `1.5px solid ${roleColor}25`,
          display: 'flex', alignItems: 'center', justifyContent: 'center',
          fontSize: 20,
        }}>
          {roleEmoji}
        </div>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ fontSize: 15, fontWeight: 700, color: 'var(--text-1)', lineHeight: 1.2 }}>
            {expert.name}
          </div>
          <div style={{
            fontSize: 10, fontWeight: 600, color: roleColor,
            textTransform: 'uppercase', letterSpacing: '0.08em', marginTop: 3,
          }}>
            {roleMeta.label}
          </div>
        </div>
        {/* Star rating */}
        <div style={{ display: 'flex', alignItems: 'center', gap: 3, flexShrink: 0 }}>
          <Star size={13} fill="#f59e0b" color="#f59e0b" />
          <span style={{ fontSize: 13, fontWeight: 800, color: '#f59e0b' }}>{expert.rating.toFixed(1)}</span>
        </div>
      </div>

      {/* Description */}
      <p style={{
        fontSize: 12, color: 'var(--text-3)', lineHeight: 1.5,
        margin: 0,
        display: '-webkit-box',
        WebkitLineClamp: 2,
        WebkitBoxOrient: 'vertical',
        overflow: 'hidden',
      }}>
        {expert.description}
      </p>

      {/* Model info */}
      <div style={{
        display: 'flex', alignItems: 'center', gap: 8,
        padding: '7px 10px',
        background: 'var(--bg)',
        border: '1px solid var(--border)',
        borderRadius: 4,
      }}>
        <span style={{
          width: 8, height: 8, borderRadius: '50%',
          background: providerColor,
          flexShrink: 0,
        }} />
        <span style={{ fontSize: 12, color: 'var(--text-2)', flex: 1 }}>
          {expert.modelName}
        </span>
        <span style={{ fontSize: 11, color: 'var(--text-3)' }}>
          {expert.providerName}
        </span>
      </div>

      {/* Stats row */}
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', fontSize: 11 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 4, color: 'var(--text-3)' }}>
          <Play size={10} color="var(--text-4)" />
          <span>{fmt(expert.totalRuns)} runs</span>
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: 2 }}>
          {[1, 2, 3, 4, 5].map(i => (
            <Star
              key={i}
              size={10}
              fill={i <= Math.round(expert.rating) ? '#f59e0b' : 'none'}
              color={i <= Math.round(expert.rating) ? '#f59e0b' : 'var(--text-4)'}
            />
          ))}
        </div>
      </div>

      {/* Specialization tags */}
      <div style={{ display: 'flex', flexWrap: 'wrap', gap: 4 }}>
        {expert.specializations.slice(0, 4).map(tag => (
          <span key={tag} style={{
            padding: '2px 8px', borderRadius: 99, fontSize: 10, fontWeight: 500,
            background: 'var(--bg-elevated)', color: 'var(--text-3)',
            border: '1px solid var(--border)',
          }}>
            {tag}
          </span>
        ))}
        {expert.specializations.length > 4 && (
          <span style={{
            padding: '2px 8px', borderRadius: 99, fontSize: 10, fontWeight: 500,
            background: 'var(--bg-elevated)', color: 'var(--text-4)',
            border: '1px solid var(--border)',
          }}>
            +{expert.specializations.length - 4}
          </span>
        )}
      </div>

      {/* Deploy button */}
      <div style={{ marginTop: 'auto', paddingTop: 4, borderTop: '1px solid var(--border)' }}>
        <Link href="/experts/deploy" style={{ textDecoration: 'none' }}>
          <button style={{
            width: '100%', display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 6,
            padding: '9px 14px', borderRadius: 7, cursor: 'pointer',
            border: `1.5px solid ${SECTION_COLOR}50`,
            background: `${SECTION_COLOR}12`,
            color: SECTION_COLOR, fontSize: 12, fontWeight: 700,
            transition: 'all 0.15s',
          }}>
            <Copy size={11} />
            Deploy Clone
          </button>
        </Link>
      </div>
    </motion.div>
  );
}

/* ═══════════════════════════════════════════════════════
   Main Page Component
   ═══════════════════════════════════════════════════════ */
export default function ExpertsPageWrapper() {
  return (
    <Suspense fallback={<div style={{ padding: 28 }}><Loader2 size={24} style={{ animation: 'spin 1s linear infinite' }} /></div>}>
      <ExpertsPage />
    </Suspense>
  );
}

function ExpertsPage() {
  const searchParams = useSearchParams();
  const tabParam = searchParams.get('tab') as TabKey | null;
  const highlightParam = searchParams.get('highlight');

  const [activeTab, setActiveTab] = useState<TabKey>(tabParam === 'marketplace' ? 'marketplace' : 'mine');
  const [highlightedId, setHighlightedId] = useState<string | null>(highlightParam);

  /* Mine tab state */
  const [mineSearch, setMineSearch]           = useState('');
  const [mineStatusFilter, setMineFilter]     = useState<StatusFilter>('all');
  const [mineSortBy, setMineSortBy]           = useState<SortOption>('rating');
  const [mineSortOpen, setMineSortOpen]       = useState(false);
  const [configureExpert, setConfigureExpert] = useState<Record<string, unknown> | null>(null);
  const [statsExpert, setStatsExpert]         = useState<Record<string, unknown> | null>(null);

  /* Import/Export state */
  const [showSharedImport, setShowSharedImport] = useState(false);

  /* Marketplace tab state */
  const [mpSearch, setMpSearch]         = useState('');
  const [mpRoleFilter, setMpRoleFilter] = useState<ExpertRole | 'all'>('all');
  const [mpSortBy, setMpSortBy]         = useState<MarketplaceSortOption>('rating');
  const [mpViewMode, setMpViewMode]     = useState<'graph' | 'list'>('list');

  const router = useRouter();
  const { experts, total, isLoading, mutate } = useExperts();
  const { edges: graphEdges, mutate: mutateGraph } = usePrismGraph();
  const { edges: mpGraphEdges } = useMarketplaceGraph();
  const [viewMode, setViewMode] = useState<'graph' | 'list'>('list');
  const [expertRunStatus, setExpertRunStatus] = useState<Record<string, 'running' | 'success' | 'error'>>({});
  const [deleteTarget, setDeleteTarget] = useState<Record<string, unknown> | null>(null);
  const [deleting, setDeleting] = useState(false);

  // Embed marketplace experts into Qdrant on first load (idempotent upsert)
  const mpEmbedded = useRef(false);
  useEffect(() => {
    if (mpEmbedded.current) return;
    mpEmbedded.current = true;
    fetch('/api/experts/graph/embed-marketplace', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ experts: MARKETPLACE_EXPERTS }),
    }).catch(() => {/* non-critical */});
  }, []);

  // Poll for running expert runs — survives page refresh
  const { data: runningRunsData } = useSWR<{ runs: Array<{ id: string; expertId: string; status: string }> }>(
    '/api/experts/run?status=running', fetcher, { refreshInterval: 3000 }
  );
  useEffect(() => {
    const runs = runningRunsData?.runs ?? [];
    if (runs.length > 0) {
      setExpertRunStatus(prev => {
        const next = { ...prev };
        for (const r of runs) next[r.expertId] = 'running';
        return next;
      });
    }
  }, [runningRunsData]);

  // Also poll for recently completed runs to clear running status
  const { data: recentRunsData } = useSWR<{ runs: Array<{ id: string; expertId: string; status: string }> }>(
    '/api/experts/run?status=completed', fetcher, { refreshInterval: 5000 }
  );
  useEffect(() => {
    const completed = recentRunsData?.runs ?? [];
    if (completed.length > 0) {
      setExpertRunStatus(prev => {
        const next = { ...prev };
        let changed = false;
        for (const r of completed) {
          if (next[r.expertId] === 'running') {
            next[r.expertId] = 'success';
            changed = true;
            setTimeout(() => setExpertRunStatus(p => { const n = { ...p }; delete n[r.expertId]; return n; }), 5000);
          }
        }
        return changed ? next : prev;
      });
      mutate(); // refresh expert stats
    }
  }, [recentRunsData, mutate]);

  const highlightRef = useRef<HTMLDivElement>(null);


  /* Delete expert */
  const handleDeleteExpert = (expert: Record<string, unknown>) => {
    setDeleteTarget(expert);
  };

  const confirmDelete = async () => {
    if (!deleteTarget) return;
    setDeleting(true);
    try {
      const resp = await fetch(`/api/experts?id=${deleteTarget.id}`, { method: 'DELETE' });
      if (resp.ok) {
        mutate();
        setTimeout(() => mutateGraph(), 1000); // refresh graph after Qdrant cleanup
        fetch('/api/logs', { method: 'POST', headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ level: 'info', message: `Expert deleted: ${deleteTarget.name}`, source: 'expert', metadata: { expertId: deleteTarget.id } }),
        }).catch(() => {});
      }
    } catch { /* ignore */ }
    setDeleting(false);
    setDeleteTarget(null);
  };

  /* Run expert — delegates to server-side API, survives page refresh */
  const handleRunExpert = async (expert: Record<string, unknown>) => {
    const expertId = expert.id as string;
    const expertName = expert.name as string;
    const role = expert.role as string;
    const localConfig = expert.localModelConfig as Record<string, string> | null;
    const engine = localConfig?.engine || 'ollama';
    const model = localConfig?.modelName || localConfig?.model || 'llama3.2:3b';

    setExpertRunStatus(prev => ({ ...prev, [expertId]: 'running' }));

    try {
      const resp = await fetch('/api/experts/run', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          expertId,
          expertName,
          model,
          engine,
          temperature: Number(expert.temperature) || 0.7,
          maxTokens: (expert.maxTokens as number) || 4096,
          systemPrompt: (expert.systemPrompt as string) || `You are ${expertName}, a specialized ${role} AI expert.`,
          userPrompt: `You are running as expert "${expertName}" with role "${role}". Provide a demonstration of your capabilities. Show your best work in your area of expertise with a practical example.`,
          role,
          tags: [role, 'demo', 'auto-run'],
        }),
      });

      if (!resp.ok) {
        throw new Error(`Server returned ${resp.status}`);
      }

      // Refresh expert list immediately so status shows "running"
      mutate();

      fetch('/api/logs', { method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          level: 'info', message: `Expert "${expertName}" run started (server-side)`,
          source: 'expert', metadata: { expertId, model, engine },
        }),
      }).catch(() => {});

    } catch (err) {
      fetch('/api/logs', { method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          level: 'error',
          message: `Expert "${expertName}" run failed to start: ${err instanceof Error ? err.message : 'Unknown'}`,
          source: 'expert', metadata: { expertId },
        }),
      }).catch(() => {});
      setExpertRunStatus(prev => ({ ...prev, [expertId]: 'error' }));
      setTimeout(() => setExpertRunStatus(prev => { const n = { ...prev }; delete n[expertId]; return n; }), 8000);
    }
  };

  /* Sync tab from URL params — intentional setState from param changes */
  useEffect(() => {
    if (tabParam === 'marketplace') setActiveTab('marketplace');
    else if (tabParam === 'mine' || !tabParam) setActiveTab('mine');
  }, [tabParam]);

  /* Highlight behavior */
  useEffect(() => {
    if (!highlightedId) return;

    const timer = setTimeout(() => {
      if (highlightRef.current) {
        highlightRef.current.scrollIntoView({ behavior: 'smooth', block: 'center' });
      }
    }, 300);

    const clearTimer = setTimeout(() => {
      setHighlightedId(null);
      /* Clear highlight from URL without navigation */
      const url = new URL(window.location.href);
      url.searchParams.delete('highlight');
      window.history.replaceState({}, '', url.toString());
    }, 3000);

    return () => {
      clearTimeout(timer);
      clearTimeout(clearTimer);
    };
  }, [highlightedId]);

  /* Tab switching with URL update */
  const switchTab = (tab: TabKey) => {
    setActiveTab(tab);
    const url = new URL(window.location.href);
    url.searchParams.set('tab', tab);
    url.searchParams.delete('highlight');
    window.history.replaceState({}, '', url.toString());
  };

  /* ─── My Experts derived data ──────────────────────── */
  const activeCt    = experts.filter((e: Record<string, unknown>) => e.status === 'active').length;
  const fineTunedCt = experts.filter((e: Record<string, unknown>) => e.isFinetuned).length;
  const avgSuccess  = useMemo(() => {
    const rates = experts
      .map((e: Record<string, unknown>) => {
        const s = e.stats as Record<string, number> | undefined;
        return (e.successRate as number) ?? s?.successRate ?? 0;
      })
      .filter((r: number) => r > 0);
    if (rates.length === 0) return 0;
    return rates.reduce((a: number, b: number) => a + b, 0) / rates.length;
  }, [experts]);

  const mineFiltered = useMemo(() => {
    let list = [...experts] as Record<string, unknown>[];
    if (mineStatusFilter !== 'all') {
      list = list.filter(e => e.status === mineStatusFilter);
    }
    if (mineSearch.trim()) {
      const q = mineSearch.trim().toLowerCase();
      list = list.filter(e =>
        (e.name as string).toLowerCase().includes(q) ||
        (e.role as string).toLowerCase().includes(q) ||
        ((e.tags as string[]) ?? []).some(t => t.toLowerCase().includes(q)),
      );
    }
    if (mineSortBy === 'rating') {
      list.sort((a, b) => {
        const ra = (a.stats as Record<string, number>)?.rating ?? 0;
        const rb = (b.stats as Record<string, number>)?.rating ?? 0;
        return rb - ra;
      });
    } else if (mineSortBy === 'runs') {
      list.sort((a, b) => {
        const ra = (a.stats as Record<string, number>)?.totalRuns ?? (a.totalRuns as number) ?? 0;
        const rb = (b.stats as Record<string, number>)?.totalRuns ?? (b.totalRuns as number) ?? 0;
        return rb - ra;
      });
    } else if (mineSortBy === 'name') {
      list.sort((a, b) => (a.name as string).localeCompare(b.name as string));
    } else if (mineSortBy === 'cost') {
      list.sort((a, b) => {
        const ca = (a.stats as Record<string, number>)?.avgCostPerRun ?? 0;
        const cb = (b.stats as Record<string, number>)?.avgCostPerRun ?? 0;
        return cb - ca;
      });
    }
    return list;
  }, [experts, mineStatusFilter, mineSearch, mineSortBy]);

  const currentMineSortLabel = SORT_OPTIONS.find(o => o.value === mineSortBy)?.label ?? 'Sort';

  /* ─── Marketplace derived data ─────────────────────── */
  const ALL_MP_ROLES: ExpertRole[] = [
    'researcher', 'coder', 'writer', 'data-engineer', 'legal',
    'financial', 'creative', 'reviewer', 'coordinator', 'medical',
    'planner', 'translator',
  ];

  const mpFiltered = useMemo(() => {
    let list = [...MARKETPLACE_EXPERTS];
    if (mpRoleFilter !== 'all') {
      list = list.filter(e => e.role === mpRoleFilter);
    }
    if (mpSearch.trim()) {
      const q = mpSearch.trim().toLowerCase();
      list = list.filter(e =>
        e.name.toLowerCase().includes(q) ||
        e.description.toLowerCase().includes(q) ||
        e.specializations.some(s => s.toLowerCase().includes(q)),
      );
    }
    if (mpSortBy === 'rating') {
      list.sort((a, b) => b.rating - a.rating);
    } else if (mpSortBy === 'runs') {
      list.sort((a, b) => b.totalRuns - a.totalRuns);
    } else if (mpSortBy === 'name') {
      list.sort((a, b) => a.name.localeCompare(b.name));
    }
    return list;
  }, [mpRoleFilter, mpSearch, mpSortBy]);

  return (
    <div style={{ padding: 28, maxWidth: 1200 }}>
      {/* Delete Confirmation Dialog */}
      <AnimatePresence>
        {deleteTarget && (
          <motion.div
            initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }}
            style={{
              position: 'fixed', inset: 0, background: 'rgba(7,7,26,0.85)',
              zIndex: 200, display: 'flex', alignItems: 'center', justifyContent: 'center',
            }}
            onClick={() => { if (!deleting) setDeleteTarget(null); }}
          >
            <motion.div
              initial={{ opacity: 0, scale: 0.96, y: -8 }}
              animate={{ opacity: 1, scale: 1, y: 0 }}
              exit={{ opacity: 0, scale: 0.96, y: -8 }}
              transition={{ type: 'spring', damping: 25, stiffness: 300 }}
              onClick={e => e.stopPropagation()}
              style={{
                width: 420, background: 'var(--bg-surface)', border: '1px solid var(--border-md)',
                borderRadius: 12, overflow: 'hidden', boxShadow: '0 24px 80px rgba(0,0,0,0.2)',
              }}
            >
              <div style={{ padding: '20px 24px', borderBottom: '1px solid var(--border)' }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                  <div style={{
                    width: 36, height: 36, borderRadius: 8,
                    background: 'rgba(239,68,68,0.08)', border: '1px solid rgba(239,68,68,0.15)',
                    display: 'flex', alignItems: 'center', justifyContent: 'center',
                  }}>
                    <Trash2 size={16} color="#ef4444" />
                  </div>
                  <div>
                    <div style={{ fontSize: 15, fontWeight: 700, color: 'var(--text-1)' }}>Delete PRISM</div>
                    <div style={{ fontSize: 12, color: 'var(--text-3)', marginTop: 1 }}>This action cannot be undone</div>
                  </div>
                </div>
              </div>
              <div style={{ padding: '16px 24px' }}>
                <p style={{ fontSize: 13, color: 'var(--text-2)', lineHeight: 1.6, margin: 0 }}>
                  Are you sure you want to delete <strong style={{ color: 'var(--text-1)' }}>{deleteTarget.name as string}</strong>?
                  All configuration, run history, and associated artifacts will be permanently removed.
                </p>
              </div>
              <div style={{
                padding: '14px 24px', borderTop: '1px solid var(--border)',
                display: 'flex', justifyContent: 'flex-end', gap: 8,
              }}>
                <button
                  onClick={() => setDeleteTarget(null)}
                  disabled={deleting}
                  style={{
                    padding: '8px 16px', borderRadius: 7, fontSize: 12,
                    border: '1px solid var(--border-md)', background: 'transparent',
                    color: 'var(--text-3)', cursor: 'pointer',
                  }}
                >
                  Cancel
                </button>
                <button
                  onClick={confirmDelete}
                  disabled={deleting}
                  style={{
                    display: 'flex', alignItems: 'center', gap: 5,
                    padding: '8px 18px', borderRadius: 7, fontSize: 12, fontWeight: 700,
                    border: '1.5px solid #ef4444', background: 'rgba(239,68,68,0.1)',
                    color: '#ef4444', cursor: deleting ? 'wait' : 'pointer',
                    opacity: deleting ? 0.6 : 1,
                  }}
                >
                  {deleting ? <Loader2 size={12} style={{ animation: 'spin 1s linear infinite' }} /> : <Trash2 size={12} />}
                  {deleting ? 'Deleting...' : 'Delete PRISM'}
                </button>
              </div>
            </motion.div>
          </motion.div>
        )}
      </AnimatePresence>

      {/* Highlight animation keyframes */}
      <style>{`
        @keyframes highlight-pulse {
          0%, 100% { box-shadow: 0 0 0 0 transparent; }
          50% { box-shadow: 0 0 0 4px ${SECTION_COLOR}40; }
        }
      `}</style>

      {/* ── Header ── */}
      <motion.div
        initial={{ opacity: 0, y: -8 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.3 }}
        style={{
          display: 'flex', alignItems: 'center',
          justifyContent: 'space-between', marginBottom: 20,
        }}
      >
        <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
          <div style={{
            width: 38, height: 38, borderRadius: 9,
            background: `${SECTION_COLOR}15`,
            border: `1.5px solid ${SECTION_COLOR}30`,
            display: 'flex', alignItems: 'center', justifyContent: 'center',
          }}>
            <Star size={19} color={SECTION_COLOR} strokeWidth={2} />
          </div>
          <div>
            <h1 style={{ fontSize: 18, fontWeight: 700, color: 'var(--text-1)', lineHeight: 1, margin: 0 }}>
              PRISM
            </h1>
            <p style={{ fontSize: 10, color: 'var(--text-4)', marginTop: 3, margin: '3px 0 0', letterSpacing: '0.5px', textTransform: 'uppercase' }}>
              Prompt &middot; References &middot; Instructions &middot; Scripts &middot; Models
            </p>
            <p style={{ fontSize: 12, color: 'var(--text-3)', marginTop: 4, margin: '4px 0 0', maxWidth: 420 }}>
              Build autonomous AI specialists — chain reasoning, code generation,
              and domain expertise into intelligent agents that collaborate, learn,
              and evolve with every run.
            </p>
          </div>
        </div>

        <div style={{ display: 'flex', gap: 8 }}>
          <ImportButton entityType="expert" onImported={() => mutate()} size="md" />
          <SharedImportButton onClick={() => setShowSharedImport(true)} size="md" />
          <Link href="/experts/deploy" style={{
            display: 'flex', alignItems: 'center', gap: 6,
            padding: '8px 15px', borderRadius: 8,
            border: `1.5px solid ${SECTION_COLOR}`,
            background: `${SECTION_COLOR}14`,
            color: SECTION_COLOR, fontSize: 12, fontWeight: 700,
            textDecoration: 'none',
          }}>
            <Plus size={13} strokeWidth={2.5} />
            Create New
          </Link>
        </div>
      </motion.div>

      {/* ── Tabs ── */}
      <div style={{
        display: 'flex', gap: 0, marginBottom: 24,
        borderBottom: '1px solid var(--border)',
      }}>
        {([
          { key: 'mine' as TabKey, label: 'My PRISMs', icon: Star, count: total },
          { key: 'marketplace' as TabKey, label: 'Marketplace', icon: Store, count: MARKETPLACE_EXPERTS.length },
        ]).map(tab => {
          const isActive = activeTab === tab.key;
          const Icon = tab.icon;
          return (
            <button
              key={tab.key}
              onClick={() => switchTab(tab.key)}
              style={{
                display: 'flex', alignItems: 'center', gap: 7,
                padding: '12px 20px', cursor: 'pointer',
                border: 'none', background: 'none',
                borderBottom: isActive ? `2px solid ${SECTION_COLOR}` : '2px solid transparent',
                color: isActive ? SECTION_COLOR : 'var(--text-3)',
                fontWeight: isActive ? 700 : 500,
                fontSize: 13,
                transition: 'all 0.15s',
                marginBottom: -1,
              }}
            >
              <Icon size={14} />
              {tab.label}
              <span style={{
                padding: '1px 7px', borderRadius: 99, fontSize: 10, fontWeight: 700,
                background: isActive ? `${SECTION_COLOR}14` : 'var(--bg-elevated)',
                color: isActive ? SECTION_COLOR : 'var(--text-4)',
                border: `1px solid ${isActive ? `${SECTION_COLOR}30` : 'var(--border)'}`,
              }}>
                {tab.count}
              </span>
            </button>
          );
        })}
      </div>

      {/* ════════════════════════════════════════════════════
         MY EXPERTS TAB
         ════════════════════════════════════════════════════ */}
      {activeTab === 'mine' && (
        <>
          {/* Stats bar */}
          <motion.div
            variants={stagger(0.06)}
            initial="hidden"
            animate="show"
            style={{
              display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)',
              gap: 10, marginBottom: 22,
            }}
          >
            {[
              { label: 'Total PRISMs',     value: String(total),             color: SECTION_COLOR, icon: Star,      sub: 'deployed'        },
              { label: 'Active',           value: String(activeCt),          color: '#10b981',     icon: Activity,  sub: 'processing'      },
              { label: 'Fine-tuned',       value: String(fineTunedCt),       color: '#f97316',     icon: Zap,       sub: 'custom models'   },
              { label: 'Avg Success Rate', value: avgSuccess > 0 ? `${(avgSuccess * 100).toFixed(1)}%` : '—', color: '#06b6d4', icon: TrendingUp, sub: 'across all PRISMs' },
            ].map(({ label, value, color, icon: Icon, sub }) => (
              <motion.div
                key={label}
                variants={fadeUp}
                whileHover={hoverLift.whileHover}
                transition={hoverLift.transition}
                style={{
                  background: 'var(--bg-surface)',
                  border: '1px solid var(--border)',
                  borderRadius: 11, padding: '15px 18px',
                  display: 'flex', alignItems: 'center', gap: 13,
                  cursor: 'default',
                }}
              >
                <div style={{
                  width: 36, height: 36, borderRadius: 8, flexShrink: 0,
                  background: `${color}12`, border: `1.5px solid ${color}22`,
                  display: 'flex', alignItems: 'center', justifyContent: 'center',
                }}>
                  <Icon size={16} color={color} strokeWidth={2} />
                </div>
                <div>
                  <div style={{ fontSize: 22, fontWeight: 800, color, lineHeight: 1 }}>{value}</div>
                  <div style={{ fontSize: 11, fontWeight: 600, color: 'var(--text-2)', marginTop: 2 }}>{label}</div>
                  <div style={{ fontSize: 10, color: 'var(--text-4)', marginTop: 1 }}>{sub}</div>
                </div>
              </motion.div>
            ))}
          </motion.div>

          {/* Search + filters + sort */}
          <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            transition={{ delay: 0.13 }}
            style={{ display: 'flex', gap: 8, flexWrap: 'wrap', alignItems: 'center', marginBottom: 20 }}
          >
            {/* Search */}
            <div style={{
              display: 'flex', alignItems: 'center', gap: 8,
              padding: '7px 13px', borderRadius: 8, border: '1px solid var(--border-md)',
              background: 'var(--bg-surface)',
            }}>
              <Search size={13} color="var(--text-4)" />
              <input
                value={mineSearch}
                onChange={e => setMineSearch(e.target.value)}
                placeholder="Search name, role, or tag..."
                style={{
                  border: 'none', outline: 'none', background: 'transparent',
                  fontSize: 13, color: 'var(--text-1)', width: 200,
                }}
              />
            </div>

            {/* Status filter tabs */}
            <div style={{ display: 'flex', gap: 5 }}>
              {STATUS_FILTERS.map(f => {
                const cnt = f === 'all'
                  ? experts.length
                  : experts.filter((e: Record<string, unknown>) => e.status === f).length;
                return (
                  <button
                    key={f}
                    onClick={() => setMineFilter(f)}
                    style={{
                      padding: '6px 13px', borderRadius: 99, fontSize: 12, cursor: 'pointer',
                      border: mineStatusFilter === f ? `1.5px solid ${SECTION_COLOR}` : '1px solid var(--border-md)',
                      background: mineStatusFilter === f ? `${SECTION_COLOR}14` : 'var(--bg-surface)',
                      color: mineStatusFilter === f ? SECTION_COLOR : 'var(--text-3)',
                      fontWeight: mineStatusFilter === f ? 700 : 400,
                      transition: 'all 0.15s',
                    }}
                  >
                    {f.charAt(0).toUpperCase() + f.slice(1)}
                    <span style={{ marginLeft: 4, fontSize: 10, opacity: 0.7 }}>({cnt})</span>
                  </button>
                );
              })}
            </div>

            {/* Sort dropdown */}
            <div style={{ position: 'relative', marginLeft: 'auto' }}>
              <button
                onClick={() => setMineSortOpen(o => !o)}
                style={{
                  display: 'flex', alignItems: 'center', gap: 6,
                  padding: '7px 13px', borderRadius: 8, border: '1px solid var(--border-md)',
                  background: 'var(--bg-surface)', cursor: 'pointer',
                  fontSize: 12, color: 'var(--text-2)', fontWeight: 500,
                }}
              >
                <Clock size={12} />
                Sort: {currentMineSortLabel}
                <ChevronDown size={11} style={{ transition: 'transform 0.15s', transform: mineSortOpen ? 'rotate(180deg)' : 'none' }} />
              </button>
              <AnimatePresence>
                {mineSortOpen && (
                  <motion.div
                    initial={{ opacity: 0, y: -6, scale: 0.97 }}
                    animate={{ opacity: 1, y: 0, scale: 1 }}
                    exit={{ opacity: 0, y: -4, scale: 0.97 }}
                    transition={{ duration: 0.14 }}
                    style={{
                      position: 'absolute', top: 'calc(100% + 6px)', right: 0, zIndex: 50,
                      background: 'var(--bg-surface)', border: '1px solid var(--border-md)',
                      borderRadius: 9, padding: 6, minWidth: 140,
                      boxShadow: '0 8px 24px rgba(13,13,13,0.10)',
                    }}
                  >
                    {SORT_OPTIONS.map(opt => (
                      <button
                        key={opt.value}
                        onClick={() => { setMineSortBy(opt.value); setMineSortOpen(false); }}
                        style={{
                          display: 'block', width: '100%', textAlign: 'left',
                          padding: '7px 12px', borderRadius: 6, cursor: 'pointer',
                          fontSize: 12, fontWeight: mineSortBy === opt.value ? 700 : 400,
                          background: mineSortBy === opt.value ? `${SECTION_COLOR}12` : 'transparent',
                          color: mineSortBy === opt.value ? SECTION_COLOR : 'var(--text-2)',
                          border: 'none',
                        }}
                      >
                        {opt.label}
                      </button>
                    ))}
                  </motion.div>
                )}
              </AnimatePresence>
            </div>

            {/* Graph / List toggle */}
            <div style={{ display: 'flex', gap: 2, marginLeft: 'auto', background: 'var(--bg-surface)', borderRadius: 8, border: '1px solid var(--border-md)', padding: 2 }}>
              {([
                { mode: 'graph' as const, icon: Network, title: 'Graph view' },
                { mode: 'list' as const, icon: LayoutGrid, title: 'List view' },
              ]).map(({ mode, icon: Icon, title }) => (
                <button
                  key={mode}
                  onClick={() => setViewMode(mode)}
                  title={title}
                  style={{
                    display: 'flex', alignItems: 'center', justifyContent: 'center',
                    width: 30, height: 28, borderRadius: 6, border: 'none',
                    background: viewMode === mode ? `${SECTION_COLOR}18` : 'transparent',
                    color: viewMode === mode ? SECTION_COLOR : 'var(--text-4)',
                    cursor: 'pointer', transition: 'all 0.15s',
                  }}
                >
                  <Icon size={14} />
                </button>
              ))}
            </div>
          </motion.div>

          {/* PRISM Graph view */}
          {viewMode === 'graph' && !isLoading && (
            <div style={{ marginBottom: 20 }}>
              <PrismGraph
                prisms={experts}
                edges={graphEdges}
                onNodeClick={(id) => {
                  const expert = experts.find((e: Record<string, unknown>) => e.id === id);
                  if (expert) setConfigureExpert(expert);
                }}
                search={mineSearch}
              />
            </div>
          )}

          {/* PRISM List view */}
          {viewMode === 'list' && !isLoading && (
            <PrismListView
              prisms={mineFiltered}
              edges={graphEdges}
              onConfigure={setConfigureExpert}
              onRun={handleRunExpert}
              onDelete={handleDeleteExpert}
              onExport={(p) => exportEntity('expert', p.id as string, p.name as string)}
              onCreateEdge={async (sourceId, targetId) => {
                try {
                  await fetch('/api/experts/graph', {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({ source: sourceId, target: targetId }),
                  });
                  mutate();
                  setTimeout(() => mutateGraph(), 2000); // refresh graph after re-embedding
                } catch { /* ignore */ }
              }}
            />
          )}

          {/* PRISM cards loading skeleton */}
          {viewMode === 'list' && isLoading && (
            <div style={{
              display: 'grid',
              gridTemplateColumns: 'repeat(auto-fill, minmax(280px, 1fr))',
              gap: 16,
            }}>
              {[0, 1, 2, 3].map(i => <SkeletonCard key={i} />)}
            </div>
          )}

          {/* Footer */}
          <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            transition={{ delay: 0.5 }}
            style={{
              marginTop: 28, display: 'flex', alignItems: 'center', gap: 8,
              color: 'var(--text-4)', fontSize: 11,
            }}
          >
            <div className="dot-pulse" style={{
              width: 6, height: 6, borderRadius: '50%', background: SECTION_COLOR,
            }} />
            Auto-refreshes every 20 seconds
            {mineFiltered.length !== total && (
              <span style={{ marginLeft: 8 }}>· Showing {mineFiltered.length} of {total}</span>
            )}
          </motion.div>
        </>
      )}

      {/* ════════════════════════════════════════════════════
         MARKETPLACE TAB
         ════════════════════════════════════════════════════ */}
      {activeTab === 'marketplace' && (
        <>
          {/* Search + filters */}
          <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            transition={{ delay: 0.08 }}
            style={{ display: 'flex', gap: 10, flexWrap: 'wrap', alignItems: 'center', marginBottom: 16 }}
          >
            {/* Search */}
            <div style={{
              display: 'flex', alignItems: 'center', gap: 8,
              padding: '7px 13px', borderRadius: 8, border: '1px solid var(--border-md)',
              background: 'var(--bg-surface)', flex: 1, minWidth: 200, maxWidth: 340,
            }}>
              <Search size={13} color="var(--text-4)" />
              <input
                value={mpSearch}
                onChange={e => setMpSearch(e.target.value)}
                placeholder="Search templates..."
                style={{
                  border: 'none', outline: 'none', background: 'transparent',
                  fontSize: 13, color: 'var(--text-1)', width: '100%',
                }}
              />
            </div>

            {/* Sort */}
            <select
              value={mpSortBy}
              onChange={e => setMpSortBy(e.target.value as MarketplaceSortOption)}
              style={{
                padding: '7px 13px', borderRadius: 8, border: '1px solid var(--border-md)',
                background: 'var(--bg-surface)', fontSize: 12, color: 'var(--text-2)',
                cursor: 'pointer', outline: 'none',
              }}
            >
              <option value="rating">Sort: Rating</option>
              <option value="runs">Sort: Popularity</option>
              <option value="name">Sort: Name</option>
            </select>

            {/* Graph / List toggle */}
            <div style={{ display: 'flex', gap: 2, marginLeft: 'auto', background: 'var(--bg-surface)', borderRadius: 8, border: '1px solid var(--border-md)', padding: 2 }}>
              {([
                { mode: 'graph' as const, icon: Network, title: 'Graph view' },
                { mode: 'list' as const, icon: LayoutGrid, title: 'Card view' },
              ]).map(({ mode, icon: Icon, title }) => (
                <button
                  key={mode}
                  onClick={() => setMpViewMode(mode)}
                  title={title}
                  style={{
                    display: 'flex', alignItems: 'center', justifyContent: 'center',
                    width: 30, height: 28, borderRadius: 6, border: 'none',
                    background: mpViewMode === mode ? `${SECTION_COLOR}18` : 'transparent',
                    color: mpViewMode === mode ? SECTION_COLOR : 'var(--text-4)',
                    cursor: 'pointer', transition: 'all 0.15s',
                  }}
                >
                  <Icon size={14} />
                </button>
              ))}
            </div>
          </motion.div>

          {/* Role filter pills */}
          <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            transition={{ delay: 0.12 }}
            style={{ display: 'flex', gap: 6, flexWrap: 'wrap', marginBottom: 20 }}
          >
            <button
              onClick={() => setMpRoleFilter('all')}
              style={{
                padding: '5px 13px', borderRadius: 99, fontSize: 12, cursor: 'pointer',
                border: mpRoleFilter === 'all' ? `1.5px solid ${SECTION_COLOR}` : '1px solid var(--border-md)',
                background: mpRoleFilter === 'all' ? `${SECTION_COLOR}14` : 'var(--bg-surface)',
                color: mpRoleFilter === 'all' ? SECTION_COLOR : 'var(--text-3)',
                fontWeight: mpRoleFilter === 'all' ? 700 : 400,
                transition: 'all 0.15s',
              }}
            >
              All Roles
            </button>
            {ALL_MP_ROLES.map(role => {
              const meta = ROLE_META[role];
              const isActive = mpRoleFilter === role;
              const rc = ROLE_COLOR[role] ?? '#6b7280';
              return (
                <button
                  key={role}
                  onClick={() => setMpRoleFilter(isActive ? 'all' : role)}
                  style={{
                    padding: '5px 13px', borderRadius: 99, fontSize: 12, cursor: 'pointer',
                    border: isActive ? `1.5px solid ${rc}` : '1px solid var(--border-md)',
                    background: isActive ? `${rc}14` : 'var(--bg-surface)',
                    color: isActive ? rc : 'var(--text-3)',
                    fontWeight: isActive ? 700 : 400,
                    transition: 'all 0.15s',
                  }}
                >
                  {meta.emoji} {meta.label}
                </button>
              );
            })}
          </motion.div>

          {/* Results count */}
          {mpFiltered.length !== MARKETPLACE_EXPERTS.length && (
            <div style={{ fontSize: 12, color: 'var(--text-3)', marginBottom: 14 }}>
              Showing {mpFiltered.length} of {MARKETPLACE_EXPERTS.length} templates
            </div>
          )}

          {/* Marketplace graph view */}
          {mpViewMode === 'graph' && (
            <div style={{ marginBottom: 20 }}>
              <PrismGraph
                prisms={mpFiltered.map(e => ({
                  id: e.id, name: e.name, description: e.description,
                  role: e.role, status: 'active', tags: e.tags,
                  category: e.role, complexityLevel: 3,
                  totalRuns: e.totalRuns, rating: e.rating,
                }))}
                edges={mpGraphEdges}
                onNodeClick={(id) => {
                  router.push(`/experts/deploy`);
                }}
                search={mpSearch}
              />
            </div>
          )}

          {/* Marketplace card grid */}
          {mpViewMode === 'list' && (mpFiltered.length === 0 ? (
            <motion.div
              initial={{ opacity: 0, y: 12 }}
              animate={{ opacity: 1, y: 0 }}
              style={{
                textAlign: 'center', padding: '80px 0',
                display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 14,
              }}
            >
              <div style={{
                width: 56, height: 56, borderRadius: 14,
                background: 'var(--bg-elevated)', border: '1px solid var(--border)',
                display: 'flex', alignItems: 'center', justifyContent: 'center',
              }}>
                <Store size={24} color="var(--text-4)" />
              </div>
              <div>
                <div style={{ fontSize: 15, fontWeight: 600, color: 'var(--text-2)', marginBottom: 6 }}>
                  No templates match your search
                </div>
                <div style={{ fontSize: 12, color: 'var(--text-4)', maxWidth: 340 }}>
                  Try adjusting your search terms or clearing the role filter.
                </div>
              </div>
              <button
                onClick={() => { setMpSearch(''); setMpRoleFilter('all'); }}
                style={{
                  display: 'inline-flex', alignItems: 'center', gap: 6,
                  padding: '8px 16px', borderRadius: 7, cursor: 'pointer',
                  border: '1px solid var(--border-md)', background: 'var(--bg-surface)',
                  color: 'var(--text-2)', fontSize: 12, fontWeight: 500,
                }}
              >
                <RotateCcw size={12} /> Reset Filters
              </button>
            </motion.div>
          ) : (
            <motion.div
              key={`mp-${mpRoleFilter}-${mpSortBy}`}
              variants={staggerDefault}
              initial="hidden"
              animate="show"
              style={{
                display: 'grid',
                gridTemplateColumns: 'repeat(auto-fill, minmax(280px, 1fr))',
                gap: 16,
              }}
            >
              {mpFiltered.map(expert => (
                <MarketplaceCard key={expert.id} expert={expert} />
              ))}
            </motion.div>
          ))}
        </>
      )}

      {/* ── Modals ── */}
      <ExpertEditDialog
        expert={configureExpert as Expert | null}
        open={!!configureExpert}
        onClose={() => setConfigureExpert(null)}
        onSaved={() => { mutate(); setTimeout(() => mutateGraph(), 2000); setConfigureExpert(null); }}
      />
      <AnimatePresence>
        {statsExpert && (
          <StatsModal
            key="stats"
            expert={statsExpert}
            onClose={() => setStatsExpert(null)}
          />
        )}
      </AnimatePresence>

      {/* Shared Config Import Dialog */}
      <SharedConfigImportDialog
        open={showSharedImport}
        onClose={() => setShowSharedImport(false)}
        onImported={() => { mutate(); setShowSharedImport(false); }}
        filterType="expert"
      />
    </div>
  );
}
