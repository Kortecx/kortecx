'use client';

import { useState } from 'react';
import { motion } from 'framer-motion';
import { Send, Clock, Check, Eye } from 'lucide-react';
import { PLATFORMS } from '@/lib/constants';
import {
  fadeUp, fadeDown, stagger, buttonHover,
  rowEntrance, emptyState, filterTab,
} from '@/lib/motion';

const MOCK_PUBLICATIONS: Array<{
  id: string; content: string; platforms: string[];
  status: 'published' | 'scheduled'; publishedAt: string;
  engagement: { views: number; likes: number; shares: number };
}> = [];

export default function PublishPage() {
  const [content, setContent] = useState('');
  const [selectedPlatforms, setSelectedPlatforms] = useState<string[]>([]);

  const togglePlatform = (id: string) => {
    setSelectedPlatforms(prev =>
      prev.includes(id) ? prev.filter(p => p !== id) : [...prev, id]
    );
  };

  return (
    <div style={{ padding: 24, maxWidth: 1200, margin: '0 auto' }}>
      {/* Header */}
      <motion.div variants={fadeDown} initial="hidden" animate="show" style={{ marginBottom: 28 }}>
        <h1 style={{ fontSize: 20, fontWeight: 700, color: 'var(--text-1)', margin: 0 }}>
          Content Publishing
        </h1>
        <p style={{ fontSize: 13, color: 'var(--text-3)', margin: '4px 0 0' }}>
          Create and distribute content across platforms
        </p>
      </motion.div>

      <motion.div
        variants={stagger(0.1)}
        initial="hidden"
        animate="show"
        style={{ display: 'grid', gridTemplateColumns: '1fr 340px', gap: 16 }}
      >
        {/* Main content area */}
        <div>
          {/* Compose */}
          <motion.div variants={fadeUp} className="card" style={{ padding: 20, marginBottom: 16 }}>
            <h2 style={{ fontSize: 14, fontWeight: 600, color: 'var(--text-1)', margin: '0 0 12px' }}>
              Compose
            </h2>
            <textarea
              className="input"
              value={content}
              onChange={e => setContent(e.target.value)}
              placeholder="Write your content or generate with AI..."
              rows={6}
              style={{ width: '100%', resize: 'vertical', marginBottom: 12 }}
            />
            <div style={{ display: 'flex', gap: 8, justifyContent: 'space-between', alignItems: 'center' }}>
              <span style={{ fontSize: 11, color: 'var(--text-4)' }}>
                {content.length} characters
              </span>
              <div style={{ display: 'flex', gap: 8 }}>
                <button className="btn btn-secondary btn-sm">
                  <Clock size={12} /> Schedule
                </button>
                <motion.button
                  className="btn btn-primary btn-sm"
                  disabled={!content || selectedPlatforms.length === 0}
                  {...buttonHover}
                >
                  <Send size={12} /> Publish Now
                </motion.button>
              </div>
            </div>
          </motion.div>

          {/* Recent publications */}
          <motion.div variants={fadeUp} className="card" style={{ overflow: 'hidden' }}>
            <div style={{
              padding: '13px 20px', borderBottom: '1px solid var(--border)',
              fontSize: 14, fontWeight: 600, color: 'var(--text-1)',
            }}>
              Recent Publications
            </div>
            {MOCK_PUBLICATIONS.length === 0 ? (
              <motion.div
                {...emptyState}
                style={{
                  padding: '32px 20px',
                  textAlign: 'center',
                  fontSize: 13,
                  color: 'var(--text-4)',
                }}
              >
                No publications yet. Compose content above and publish it.
              </motion.div>
            ) : (
              MOCK_PUBLICATIONS.map((pub, i) => (
                <motion.div
                  key={pub.id}
                  {...rowEntrance(i)}
                  style={{
                    padding: '14px 20px',
                    borderBottom: i < MOCK_PUBLICATIONS.length - 1 ? '1px solid var(--border)' : 'none',
                  }}
                >
                  <div style={{ display: 'flex', alignItems: 'flex-start', gap: 12 }}>
                    <div style={{ flex: 1 }}>
                      <p style={{
                        fontSize: 13, color: 'var(--text-1)', margin: '0 0 8px',
                        lineHeight: 1.5,
                        display: '-webkit-box', WebkitLineClamp: 2,
                        WebkitBoxOrient: 'vertical', overflow: 'hidden',
                      }}>
                        {pub.content}
                      </p>
                      <div style={{ display: 'flex', alignItems: 'center', gap: 8, fontSize: 11, color: 'var(--text-3)' }}>
                        <div style={{ display: 'flex', gap: 4 }}>
                          {pub.platforms.map(pId => {
                            const platform = PLATFORMS.find(p => p.id === pId);
                            return platform ? (
                              <span key={pId} style={{
                                width: 6, height: 6, borderRadius: '50%',
                                background: platform.color,
                              }} />
                            ) : null;
                          })}
                        </div>
                        <span>{pub.platforms.map(pId => PLATFORMS.find(p => p.id === pId)?.name).join(', ')}</span>
                        <span style={{ color: 'var(--text-4)' }}>·</span>
                        <span>{new Date(pub.publishedAt).toLocaleDateString()}</span>
                      </div>
                    </div>
                    <div style={{ textAlign: 'right', flexShrink: 0 }}>
                      <span className={`badge ${pub.status === 'published' ? 'badge-success' : 'badge-amber'}`}>
                        {pub.status}
                      </span>
                      {pub.status === 'published' && (
                        <div style={{ display: 'flex', gap: 8, marginTop: 6, fontSize: 11, color: 'var(--text-3)' }}>
                          <span style={{ display: 'flex', alignItems: 'center', gap: 2 }}>
                            <Eye size={10} /> {pub.engagement.views}
                          </span>
                          <span>♥ {pub.engagement.likes}</span>
                        </div>
                      )}
                    </div>
                  </div>
                </motion.div>
              ))
            )}
          </motion.div>
        </div>

        {/* Right sidebar */}
        <div>
          {/* Platform selector */}
          <motion.div variants={fadeUp} className="card" style={{ padding: 16, marginBottom: 12 }}>
            <h3 style={{ fontSize: 13, fontWeight: 600, color: 'var(--text-1)', margin: '0 0 12px' }}>
              Select Platforms
            </h3>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
              {PLATFORMS.map(platform => {
                const selected = selectedPlatforms.includes(platform.id);
                return (
                  <motion.button
                    key={platform.id}
                    onClick={() => togglePlatform(platform.id)}
                    {...filterTab}
                    animate={selected ? { scale: [1, 1.04, 1] } : {}}
                    transition={selected
                      ? { type: 'spring', stiffness: 500, damping: 15 }
                      : filterTab.transition
                    }
                    style={{
                      display: 'flex', alignItems: 'center', gap: 10,
                      padding: '8px 12px',
                      background: selected ? platform.bgColor : 'transparent',
                      border: `1px solid ${selected ? platform.color + '40' : 'var(--border)'}`,
                      borderRadius: 5, cursor: 'pointer',
                      fontSize: 13, textAlign: 'left',
                      color: selected ? platform.color : 'var(--text-2)',
                      fontWeight: selected ? 500 : 400,
                      transition: 'background 0.12s, border-color 0.12s, color 0.12s',
                    }}
                  >
                    <span style={{
                      width: 8, height: 8, borderRadius: '50%',
                      background: platform.color, flexShrink: 0,
                    }} />
                    <span style={{ flex: 1 }}>{platform.name}</span>
                    {selected && <Check size={14} />}
                  </motion.button>
                );
              })}
            </div>
          </motion.div>

          {/* Quick stats */}
          <motion.div variants={fadeUp} className="card" style={{ padding: 16 }}>
            <h3 style={{ fontSize: 13, fontWeight: 600, color: 'var(--text-1)', margin: '0 0 12px' }}>
              Publishing Stats
            </h3>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
              {[
                { label: 'Total published', value: '24' },
                { label: 'This week', value: '5' },
                { label: 'Avg engagement', value: '847 views' },
                { label: 'Scheduled', value: '1' },
              ].map(s => (
                <div key={s.label} style={{
                  display: 'flex', justifyContent: 'space-between',
                  fontSize: 12,
                }}>
                  <span style={{ color: 'var(--text-3)' }}>{s.label}</span>
                  <span className="mono" style={{ fontWeight: 600, color: 'var(--text-1)' }}>{s.value}</span>
                </div>
              ))}
            </div>
          </motion.div>
        </div>
      </motion.div>
    </div>
  );
}
