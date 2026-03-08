// Type declarations for font modules rewritten at build time
interface FontConfig {
  weight?: string | string[];
  subsets?: string[];
  display?: string;
  variable?: string;
  fallback?: string[];
}

interface FontResult {
  className: string;
  style: { fontFamily: string };
  variable?: string;
}

declare module 'next/font/google' {
  export function Inter(config?: FontConfig): FontResult;
  export function Roboto(config?: FontConfig): FontResult;
  export function Roboto_Mono(config?: FontConfig): FontResult;
}

declare module 'rex/font/google' {
  export function Inter(config?: FontConfig): FontResult;
  export function Roboto(config?: FontConfig): FontResult;
  export function Roboto_Mono(config?: FontConfig): FontResult;
}

declare module '@next/font/google' {
  export function Inter(config?: FontConfig): FontResult;
  export function Roboto(config?: FontConfig): FontResult;
  export function Roboto_Mono(config?: FontConfig): FontResult;
}
