'use client';

import { motion } from 'framer-motion';
import { buttonHover } from '@/lib/motion';
import { Sparkles, Cloud, ExternalLink, Lock, Zap, Globe, Server, Shield } from 'lucide-react';

const KORTECX_CLOUD_URL = 'https://www.kortecx.com';

export default function InferencePage() {
  return (
    <div style={{ padding: 20, maxWidth: '100%' }}>
      {/* Header */}
      <motion.div initial={{ opacity: 0, y: -8 }} animate={{ opacity: 1, y: 0 }}
        style={{ marginBottom: 24 }}>
        <h1 style={{ fontSize: 18, fontWeight: 700, color: 'var(--text-1)', margin: 0, display: 'flex', alignItems: 'center', gap: 8 }}>
          <Sparkles size={18} color="#7C3AED" /> Inference
        </h1>
        <p style={{ fontSize: 12, color: 'var(--text-3)', margin: '3px 0 0' }}>
          Managed inference endpoints for production workloads
        </p>
      </motion.div>

      {/* Cloud CTA */}
      <motion.div initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }} transition={{ delay: 0.05 }}
        className="card" style={{ padding: 0, overflow: 'hidden', marginBottom: 20 }}>
        <div style={{
          background: 'linear-gradient(135deg, rgba(124,58,237,0.08) 0%, rgba(37,99,235,0.08) 100%)',
          padding: '40px 32px', textAlign: 'center',
        }}>
          <div style={{
            width: 56, height: 56, borderRadius: 14, margin: '0 auto 16px',
            background: 'rgba(124,58,237,0.1)', border: '1.5px solid rgba(124,58,237,0.2)',
            display: 'flex', alignItems: 'center', justifyContent: 'center',
          }}>
            <Cloud size={26} color="#7C3AED" />
          </div>
          <h2 style={{ fontSize: 20, fontWeight: 800, color: 'var(--text-1)', margin: '0 0 8px' }}>
            Managed Inference on Kortecx Cloud
          </h2>
          <p style={{ fontSize: 13, color: 'var(--text-3)', lineHeight: 1.6, maxWidth: 520, margin: '0 auto 20px' }}>
            Deploy models to dedicated GPU endpoints with auto-scaling, load balancing, and 99.9% uptime SLA.
            Available on Kortecx Cloud for enterprise and production workloads.
          </p>
          <motion.a {...buttonHover} href={KORTECX_CLOUD_URL} target="_blank" rel="noopener noreferrer"
            className="btn btn-primary" style={{ fontSize: 13, padding: '10px 24px', textDecoration: 'none', display: 'inline-flex' }}>
            <ExternalLink size={14} /> Go to Kortecx Cloud
          </motion.a>
        </div>
      </motion.div>

      {/* Features grid */}
      <motion.div initial={{ opacity: 0 }} animate={{ opacity: 1 }} transition={{ delay: 0.1 }}
        style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 14 }}>
        {[
          { icon: Zap, label: 'Auto-Scaling', desc: 'Scale from 0 to thousands of requests per second based on demand', color: '#D97706' },
          { icon: Globe, label: 'Global Edge', desc: 'Deploy to 30+ regions for low-latency inference worldwide', color: '#2563EB' },
          { icon: Server, label: 'Dedicated GPUs', desc: 'A100, H100, and L40S GPU instances with reserved capacity', color: '#059669' },
          { icon: Shield, label: 'Enterprise SLA', desc: '99.9% uptime guarantee with 24/7 support and monitoring', color: '#7C3AED' },
          { icon: Lock, label: 'Data Privacy', desc: 'SOC 2 compliant, data never leaves your VPC, HIPAA ready', color: '#DC2626' },
          { icon: Sparkles, label: 'Model Registry', desc: 'Version, tag, and deploy any model from a central registry', color: '#EC4899' },
        ].map((feat, i) => (
          <motion.div key={feat.label} initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }} transition={{ delay: 0.12 + i * 0.04 }}
            className="card" style={{ padding: 20, opacity: 0.7 }}>
            <feat.icon size={18} color={feat.color} style={{ marginBottom: 10 }} />
            <div style={{ fontSize: 13, fontWeight: 650, color: 'var(--text-1)', marginBottom: 4 }}>{feat.label}</div>
            <div style={{ fontSize: 11, color: 'var(--text-3)', lineHeight: 1.5 }}>{feat.desc}</div>
          </motion.div>
        ))}
      </motion.div>

      {/* Disabled overlay info */}
      <motion.div initial={{ opacity: 0 }} animate={{ opacity: 1 }} transition={{ delay: 0.25 }}
        style={{ marginTop: 20, padding: '14px 18px', borderRadius: 8, background: 'rgba(124,58,237,0.05)', border: '1px solid rgba(124,58,237,0.15)', display: 'flex', alignItems: 'center', gap: 12 }}>
        <Lock size={16} color="#7C3AED" />
        <div>
          <div style={{ fontSize: 12, fontWeight: 600, color: 'var(--text-1)' }}>Managed inference is a cloud-only feature</div>
          <div style={{ fontSize: 11, color: 'var(--text-3)', marginTop: 2 }}>
            For local inference, use Ollama or llama.cpp via the <a href="/workflow/builder" style={{ color: '#7C3AED' }}>Workflow Builder</a> or <a href="/settings" style={{ color: '#7C3AED' }}>Settings</a> page.
          </div>
        </div>
      </motion.div>
    </div>
  );
}
