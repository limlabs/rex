"use client";

import React, { createContext, useContext, useState } from 'react';

export interface AuthContextValue {
  user: string | null;
  login: (user: string) => void;
  logout: () => void;
}

export const AuthContext = createContext<AuthContextValue>({
  user: null,
  login: () => {},
  logout: () => {},
});

export function useAuth() {
  return useContext(AuthContext);
}

export function AuthProvider({
  initialUser = null,
  children,
}: {
  initialUser?: string | null;
  children: React.ReactNode;
}) {
  const [user, setUser] = useState<string | null>(initialUser);

  return (
    <AuthContext.Provider
      value={{
        user,
        login: (u: string) => setUser(u),
        logout: () => setUser(null),
      }}
    >
      {children}
    </AuthContext.Provider>
  );
}

export default AuthProvider;
