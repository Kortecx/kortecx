'use client';

import { createContext, useContext, useState, useCallback, ReactNode } from 'react';

interface NavContextType {
  isCollapsed: boolean;
  toggleCollapse: () => void;
  activeProjectId: string | null;
  setActiveProjectId: (id: string | null) => void;
}

const NavContext = createContext<NavContextType>({
  isCollapsed: false,
  toggleCollapse: () => {},
  activeProjectId: null,
  setActiveProjectId: () => {},
});

export function NavProvider({ children }: { children: ReactNode }) {
  const [isCollapsed, setIsCollapsed] = useState(false);
  const [activeProjectId, setActiveProjectId] = useState<string | null>(null);

  const toggleCollapse = useCallback(() => {
    setIsCollapsed((prev) => !prev);
  }, []);

  return (
    <NavContext.Provider value={{ isCollapsed, toggleCollapse, activeProjectId, setActiveProjectId }}>
      {children}
    </NavContext.Provider>
  );
}

export function useNav() {
  return useContext(NavContext);
}
