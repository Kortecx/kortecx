'use client';

import {
  createContext, useContext, useState, useCallback, useEffect, ReactNode, Dispatch, SetStateAction
} from 'react';
import type { Expert, Workflow, WorkflowStep, QueuedTask, SocialPlatform, VoiceState, VoiceCommand, ContentItem } from '@/lib/types';
import { ACTIVE_TASKS, EXPERTS, WORKFLOWS, SYSTEM_METRICS } from '@/lib/constants';

interface AppContextType {
  /* System metrics */
  metrics: typeof SYSTEM_METRICS;
  refreshMetrics: () => void;

  /* Active tasks */
  activeTasks: QueuedTask[];
  addTask: (task: QueuedTask) => void;
  cancelTask: (taskId: string) => void;

  /* Expert catalog */
  experts: Expert[];
  selectedExpertIds: string[];
  toggleExpert: (expertId: string) => void;
  clearExpertSelection: () => void;

  /* Workflow builder */
  draftWorkflow: Partial<Workflow> | null;
  setDraftWorkflow: (wf: Partial<Workflow> | null) => void;
  draftSteps: WorkflowStep[];
  addDraftStep: (expertId: string) => void;
  removeDraftStep: (stepId: string) => void;
  reorderDraftSteps: (steps: WorkflowStep[]) => void;
  clearDraftWorkflow: () => void;

  /* Workflows */
  workflows: Workflow[];

  /* UI state */
  sidebarCollapsed: boolean;
  toggleSidebar: () => void;
  activeTaskPanelOpen: boolean;
  setActiveTaskPanelOpen: (open: boolean) => void;

  /* Social platforms */
  togglePlatformConnection: (platformId: string) => void;
  platforms: SocialPlatform[];

  /* Publish panel */
  publishPanelOpen: boolean;
  setPublishPanelOpen: (open: boolean) => void;
  generatedContent: ContentItem | null;
  setGeneratedContent: (c: ContentItem | null) => void;
  selectedPublishPlatforms: string[];
  setSelectedPublishPlatforms: (ids: string[]) => void;
  togglePublishPlatform: (platformId: string) => void;
  transcript: string;
  setTranscript: Dispatch<SetStateAction<string>>;
  interimTranscript: string;
  setInterimTranscript: (t: string) => void;

  /* Voice */
  voiceState: VoiceState;
  setVoiceState: (state: VoiceState) => void;
  commandHistory: VoiceCommand[];
  addCommand: (cmd: VoiceCommand) => void;
}

const AppContext = createContext<AppContextType>({} as AppContextType);

let _stepCounter = 100;

export function AppProvider({ children }: { children: ReactNode }) {
  const [metrics] = useState(SYSTEM_METRICS);
  const [activeTasks, setActiveTasks] = useState<QueuedTask[]>([]);
  const [experts] = useState<Expert[]>([]);
  const [workflows] = useState<Workflow[]>([]);
  const [selectedExpertIds, setSelectedExpertIds] = useState<string[]>([]);
  const [draftWorkflow, setDraftWorkflow] = useState<Partial<Workflow> | null>(null);
  const [draftSteps, setDraftSteps] = useState<WorkflowStep[]>([]);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);

  useEffect(() => {
    const stored = localStorage.getItem('kortecx-sidebar-collapsed');
    if (stored === 'true') {
      setSidebarCollapsed(true);
    }
  }, []);
  const [activeTaskPanelOpen, setActiveTaskPanelOpen] = useState(false);

  const refreshMetrics = useCallback(() => {
    /* In production: fetch /api/monitoring/metrics */
  }, []);

  const addTask = useCallback((task: QueuedTask) => {
    setActiveTasks(prev => [task, ...prev]);
  }, []);

  const cancelTask = useCallback((taskId: string) => {
    setActiveTasks(prev =>
      prev.map(t => t.id === taskId ? { ...t, status: 'cancelled' as const } : t)
    );
  }, []);

  const toggleExpert = useCallback((expertId: string) => {
    setSelectedExpertIds(prev =>
      prev.includes(expertId)
        ? prev.filter(id => id !== expertId)
        : [...prev, expertId]
    );
  }, []);

  const clearExpertSelection = useCallback(() => {
    setSelectedExpertIds([]);
  }, []);

  const addDraftStep = useCallback((expertId: string) => {
    const expert = experts.find(e => e.id === expertId);
    if (!expert) return;
    _stepCounter++;
    const newStep: WorkflowStep = {
      id: `step-${_stepCounter}`,
      order: draftSteps.length + 1,
      expertId: expert.id,
      expertName: expert.name,
      expertRole: expert.role,
      taskDescription: '',
      connectionType: 'sequential',
      modelSource: 'provider',
      status: 'pending',
    };
    setDraftSteps(prev => [...prev, newStep]);
  }, [draftSteps.length, experts]);

  const removeDraftStep = useCallback((stepId: string) => {
    setDraftSteps(prev =>
      prev
        .filter(s => s.id !== stepId)
        .map((s, i) => ({ ...s, order: i + 1 }))
    );
  }, []);

  const reorderDraftSteps = useCallback((steps: WorkflowStep[]) => {
    setDraftSteps(steps.map((s, i) => ({ ...s, order: i + 1 })));
  }, []);

  const clearDraftWorkflow = useCallback(() => {
    setDraftWorkflow(null);
    setDraftSteps([]);
  }, []);

  const [platforms] = useState<SocialPlatform[]>([]);
  const [publishPanelOpen, setPublishPanelOpen] = useState(false);
  const [generatedContent, setGeneratedContent] = useState<ContentItem | null>(null);
  const [selectedPublishPlatforms, setSelectedPublishPlatforms] = useState<string[]>([]);
  const [transcript, setTranscript] = useState('');
  const [interimTranscript, setInterimTranscript] = useState('');
  const [voiceState, setVoiceState] = useState<VoiceState>('idle');
  const [commandHistory, setCommandHistory] = useState<VoiceCommand[]>([]);

  const addCommand = useCallback((cmd: VoiceCommand) => {
    setCommandHistory(prev => [cmd, ...prev].slice(0, 50));
  }, []);

  const togglePublishPlatform = useCallback((platformId: string) => {
    setSelectedPublishPlatforms(prev =>
      prev.includes(platformId) ? prev.filter(id => id !== platformId) : [...prev, platformId]
    );
  }, []);

  const togglePlatformConnection = useCallback((_platformId: string) => {
    /* Platform connection state managed server-side; noop stub */
  }, []);

  const toggleSidebar = useCallback(() => {
    setSidebarCollapsed(prev => {
      const next = !prev;
      localStorage.setItem('kortecx-sidebar-collapsed', String(next));
      return next;
    });
  }, []);

  return (
    <AppContext.Provider value={{
      metrics,
      refreshMetrics,
      activeTasks,
      addTask,
      cancelTask,
      experts,
      selectedExpertIds,
      toggleExpert,
      clearExpertSelection,
      draftWorkflow,
      setDraftWorkflow,
      draftSteps,
      addDraftStep,
      removeDraftStep,
      reorderDraftSteps,
      clearDraftWorkflow,
      workflows,
      sidebarCollapsed,
      toggleSidebar,
      activeTaskPanelOpen,
      setActiveTaskPanelOpen,
      togglePlatformConnection,
      platforms,
      publishPanelOpen,
      setPublishPanelOpen,
      generatedContent,
      setGeneratedContent,
      selectedPublishPlatforms,
      setSelectedPublishPlatforms,
      togglePublishPlatform,
      transcript,
      setTranscript,
      interimTranscript,
      setInterimTranscript,
      voiceState,
      setVoiceState,
      commandHistory,
      addCommand,
    }}>
      {children}
    </AppContext.Provider>
  );
}

export function useApp() {
  return useContext(AppContext);
}
