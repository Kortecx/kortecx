'use client';

import { useState } from 'react';
import { FolderOpen, Plus, Workflow, Users, Clock, MoreHorizontal } from 'lucide-react';

const MOCK_PROJECTS: Array<{
  id: string; name: string; description: string;
  status: 'active' | 'paused'; expertCount: number;
  workflowCount: number; createdAt: string;
  lastActivity: string; color: string;
}> = [];

export default function ProjectsPage() {
  const [showNew, setShowNew] = useState(false);

  const activeCount = MOCK_PROJECTS.filter(p => p.status === 'active').length;

  return (
    <div style={{ padding: 24, maxWidth: 1400, margin: '0 auto' }}>
      {/* Header */}
      <div style={{
        display: 'flex', alignItems: 'flex-start', justifyContent: 'space-between',
        marginBottom: 24,
      }}>
        <div>
          <h1 style={{ fontSize: 20, fontWeight: 700, color: 'var(--text-1)', margin: 0 }}>
            Projects
          </h1>
          <p style={{ fontSize: 13, color: 'var(--text-3)', margin: '4px 0 0' }}>
            {MOCK_PROJECTS.length} projects · {activeCount} active
          </p>
        </div>
        <button className="btn btn-primary btn-sm" onClick={() => setShowNew(true)}>
          <Plus size={13} /> Create Project
        </button>
      </div>

      {/* Project grid */}
      <div style={{
        display: 'grid',
        gridTemplateColumns: 'repeat(auto-fill, minmax(320px, 1fr))',
        gap: 12,
      }}>
        {MOCK_PROJECTS.map(project => (
          <div key={project.id} className="card" style={{ padding: 20, position: 'relative' }}>
            {/* Color accent */}
            <div style={{
              position: 'absolute', top: 0, left: 0, right: 0,
              height: 3, background: project.color,
              borderRadius: '6px 6px 0 0',
            }} />

            <div style={{ display: 'flex', alignItems: 'flex-start', justifyContent: 'space-between', marginBottom: 10 }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                <div style={{
                  width: 36, height: 36, borderRadius: 8,
                  background: `${project.color}12`,
                  border: `1px solid ${project.color}25`,
                  display: 'flex', alignItems: 'center', justifyContent: 'center',
                }}>
                  <FolderOpen size={16} color={project.color} />
                </div>
                <div>
                  <div style={{ fontSize: 14, fontWeight: 600, color: 'var(--text-1)' }}>
                    {project.name}
                  </div>
                  <span className={`badge ${project.status === 'active' ? 'badge-success' : 'badge-neutral'}`}>
                    {project.status}
                  </span>
                </div>
              </div>
              <button style={{
                background: 'none', border: 'none', cursor: 'pointer',
                color: 'var(--text-3)', padding: 2,
              }}>
                <MoreHorizontal size={16} />
              </button>
            </div>

            <p style={{
              fontSize: 12, color: 'var(--text-3)', lineHeight: 1.5,
              margin: '0 0 14px',
              display: '-webkit-box', WebkitLineClamp: 2,
              WebkitBoxOrient: 'vertical', overflow: 'hidden',
            }}>
              {project.description}
            </p>

            <div style={{
              display: 'flex', gap: 12, fontSize: 12, color: 'var(--text-2)',
              paddingTop: 12, borderTop: '1px solid var(--border)',
            }}>
              <span style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
                <Users size={12} color="var(--text-3)" /> {project.expertCount} experts
              </span>
              <span style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
                <Workflow size={12} color="var(--text-3)" /> {project.workflowCount} workflows
              </span>
              <span style={{ display: 'flex', alignItems: 'center', gap: 4, marginLeft: 'auto' }}>
                <Clock size={12} color="var(--text-4)" />
                <span style={{ color: 'var(--text-4)' }}>{project.lastActivity}</span>
              </span>
            </div>
          </div>
        ))}

        {/* New project card */}
        <div
          onClick={() => setShowNew(true)}
          style={{
            border: '1px dashed var(--border-md)',
            borderRadius: 6, padding: 20,
            display: 'flex', flexDirection: 'column',
            alignItems: 'center', justifyContent: 'center',
            gap: 10, cursor: 'pointer', minHeight: 200,
            transition: 'border-color 0.15s, background 0.15s',
          }}
          onMouseEnter={e => {
            e.currentTarget.style.borderColor = 'var(--primary)';
            e.currentTarget.style.background = 'var(--primary-dim)';
          }}
          onMouseLeave={e => {
            e.currentTarget.style.borderColor = 'var(--border-md)';
            e.currentTarget.style.background = 'transparent';
          }}
        >
          <Plus size={24} color="var(--text-3)" />
          <span style={{ fontSize: 13, fontWeight: 500, color: 'var(--text-2)' }}>Create New Project</span>
        </div>
      </div>

      {/* Create project modal */}
      {showNew && (
        <div style={{
          position: 'fixed', inset: 0, background: 'rgba(0,0,0,0.4)',
          display: 'flex', alignItems: 'center', justifyContent: 'center', zIndex: 100,
        }}>
          <div style={{
            background: 'var(--bg-surface)', borderRadius: 8,
            padding: 24, width: 440, maxWidth: '90vw',
            boxShadow: '0 20px 60px rgba(0,0,0,0.15)',
          }}>
            <h3 style={{ fontSize: 16, fontWeight: 600, color: 'var(--text-1)', margin: '0 0 16px' }}>
              Create Project
            </h3>
            <div style={{ marginBottom: 12 }}>
              <label style={{ fontSize: 12, fontWeight: 500, color: 'var(--text-2)', display: 'block', marginBottom: 4 }}>
                Project Name
              </label>
              <input className="input" placeholder="My Project" style={{ width: '100%' }} />
            </div>
            <div style={{ marginBottom: 16 }}>
              <label style={{ fontSize: 12, fontWeight: 500, color: 'var(--text-2)', display: 'block', marginBottom: 4 }}>
                Description
              </label>
              <textarea
                className="input"
                placeholder="Describe your project..."
                rows={3}
                style={{ width: '100%', resize: 'vertical' }}
              />
            </div>
            <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
              <button className="btn btn-secondary btn-sm" onClick={() => setShowNew(false)}>
                Cancel
              </button>
              <button className="btn btn-primary btn-sm" onClick={() => setShowNew(false)}>
                Create Project
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
