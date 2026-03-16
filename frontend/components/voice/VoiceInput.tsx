'use client';

import { useCallback } from 'react';
import { Mic, MicOff, Loader2, CheckCircle2, XCircle } from 'lucide-react';
import { useApp } from '@/contexts/AppContext';
import { COMMAND_SUGGESTIONS } from '@/lib/constants';

const STATE_CONFIG = {
  idle:       { icon: Mic,          color: 'var(--text-3)',  bg: 'var(--bg-elevated)', label: 'Voice Input' },
  listening:  { icon: Mic,          color: '#DC2626',        bg: 'rgba(220,38,38,0.08)', label: 'Listening...' },
  processing: { icon: Loader2,      color: '#D97706',        bg: 'rgba(217,119,6,0.08)', label: 'Processing...' },
  success:    { icon: CheckCircle2, color: '#059669',        bg: 'rgba(5,150,105,0.08)', label: 'Done' },
  error:      { icon: XCircle,      color: '#DC2626',        bg: 'rgba(220,38,38,0.08)', label: 'Error' },
};

export default function VoiceInput() {
  const {
    voiceState, setVoiceState,
    transcript, setTranscript,
    interimTranscript, setInterimTranscript,
    addCommand,
  } = useApp();

  const toggleListening = useCallback(() => {
    if (voiceState === 'listening') {
      setVoiceState('idle');
      setInterimTranscript('');
    } else {
      setVoiceState('listening');
      setTimeout(() => {
        const mockTranscript = COMMAND_SUGGESTIONS[Math.floor(Math.random() * COMMAND_SUGGESTIONS.length)];
        setTranscript(mockTranscript);
        setVoiceState('processing');
        setTimeout(() => {
          addCommand({
            id: `cmd-${Date.now()}`,
            transcript: mockTranscript,
            intent: 'generate_content',
            timestamp: new Date(),
            status: 'processed',
          });
          setVoiceState('success');
          setTimeout(() => setVoiceState('idle'), 2000);
        }, 1500);
      }, 2000);
    }
  }, [voiceState, setVoiceState, setTranscript, setInterimTranscript, addCommand]);

  const cfg = STATE_CONFIG[voiceState];
  const Icon = cfg.icon;

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
      <button
        onClick={toggleListening}
        disabled={voiceState === 'processing'}
        style={{
          display: 'flex', alignItems: 'center', gap: 10,
          padding: '10px 16px',
          background: cfg.bg,
          border: `1px solid ${voiceState === 'listening' ? '#DC2626' : 'var(--border)'}`,
          borderRadius: 6,
          cursor: voiceState === 'processing' ? 'wait' : 'pointer',
          color: cfg.color, fontSize: 13, fontWeight: 500,
          transition: 'all 0.2s', width: '100%',
        }}
      >
        <Icon size={16} style={{
          animation: voiceState === 'listening' ? 'pulse-dot 1s ease-in-out infinite' :
                     voiceState === 'processing' ? 'spin 1s linear infinite' : 'none',
        }} />
        <span>{cfg.label}</span>
        {voiceState === 'idle' && (
          <span style={{ marginLeft: 'auto', fontSize: 10, color: 'var(--text-4)' }}>⌘M</span>
        )}
      </button>

      {transcript && voiceState !== 'idle' && (
        <div style={{
          padding: '8px 12px', background: 'var(--bg)',
          border: '1px solid var(--border)', borderRadius: 4,
          fontSize: 12, color: 'var(--text-2)', fontStyle: 'italic',
        }}>
          &ldquo;{transcript}&rdquo;
        </div>
      )}

      {interimTranscript && (
        <div style={{
          padding: '6px 12px', fontSize: 12,
          color: 'var(--text-4)', fontStyle: 'italic',
        }}>
          {interimTranscript}
        </div>
      )}
    </div>
  );
}
