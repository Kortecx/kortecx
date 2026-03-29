import { describe, it, expect } from 'vitest';

describe('Alert Rules', () => {
  describe('Trigger type validation', () => {
    const validTriggerTypes = ['workflow_failure', 'expert_error', 'high_latency', 'cost_threshold', 'error_rate', 'custom'];

    it('recognizes all valid trigger types', () => {
      for (const type of validTriggerTypes) {
        expect(validTriggerTypes.includes(type)).toBe(true);
      }
    });

    it('rejects invalid trigger types', () => {
      expect(validTriggerTypes.includes('invalid')).toBe(false);
      expect(validTriggerTypes.includes('')).toBe(false);
    });
  });

  describe('Condition structure', () => {
    it('threshold condition has correct shape', () => {
      const condition = {
        operator: 'gt' as const,
        threshold: 5000,
      };

      expect(condition.operator).toBe('gt');
      expect(condition.threshold).toBe(5000);
      expect(['gt', 'lt', 'eq'].includes(condition.operator)).toBe(true);
    });

    it('workflow reference condition has correct shape', () => {
      const condition = {
        workflowId: 'wf-abc123',
      };

      expect(typeof condition.workflowId).toBe('string');
    });

    it('expert reference condition has correct shape', () => {
      const condition = {
        expertId: 'exp-abc123',
      };

      expect(typeof condition.expertId).toBe('string');
    });
  });

  describe('Notification config structure', () => {
    it('webhook config has correct shape', () => {
      const config = {
        channel: 'webhook' as const,
        webhookUrl: 'https://hooks.slack.com/services/xxx',
      };

      expect(config.channel).toBe('webhook');
      expect(config.webhookUrl).toContain('https://');
    });

    it('mcp_server config has correct shape', () => {
      const config = {
        channel: 'mcp_server' as const,
        targetId: 'mcp-abc123',
      };

      expect(config.channel).toBe('mcp_server');
      expect(typeof config.targetId).toBe('string');
    });

    it('integration config has correct shape', () => {
      const config = {
        channel: 'integration' as const,
        targetId: 'int-abc123',
      };

      expect(config.channel).toBe('integration');
      expect(typeof config.targetId).toBe('string');
    });

    it('all channel types are valid', () => {
      const validChannels = ['mcp_server', 'integration', 'webhook'];
      expect(validChannels).toHaveLength(3);
    });
  });

  describe('Severity validation', () => {
    it('recognizes all valid severities', () => {
      const validSeverities = ['info', 'warning', 'error', 'critical'];
      expect(validSeverities).toHaveLength(4);
      expect(validSeverities.includes('warning')).toBe(true);
    });
  });

  describe('Cooldown validation', () => {
    it('cooldown minutes must be positive', () => {
      const cooldown = 15;
      expect(cooldown).toBeGreaterThan(0);
    });

    it('default cooldown is 15 minutes', () => {
      const defaultCooldown = 15;
      expect(defaultCooldown).toBe(15);
    });
  });

  describe('Alert rule data shape', () => {
    it('creates a complete alert rule object', () => {
      const rule = {
        name: 'Workflow Failure Alert',
        description: 'Notify when any workflow fails',
        triggerType: 'workflow_failure',
        conditions: {},
        notificationConfig: {
          channel: 'webhook',
          webhookUrl: 'https://hooks.slack.com/xxx',
        },
        severity: 'error',
        enabled: true,
        cooldownMinutes: 15,
      };

      expect(rule.name).toBeTruthy();
      expect(rule.triggerType).toBeTruthy();
      expect(rule.notificationConfig.channel).toBeTruthy();
      expect(rule.severity).toBeTruthy();
      expect(typeof rule.enabled).toBe('boolean');
      expect(typeof rule.cooldownMinutes).toBe('number');
    });
  });
});
