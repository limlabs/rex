export interface RouterEvents {
    on(event: string, handler: (...args: unknown[]) => void): void;
    off(event: string, handler: (...args: unknown[]) => void): void;
    emit(event: string, ...args: unknown[]): void;
}
export interface RexRouter {
    pathname: string;
    asPath: string;
    query: Record<string, string>;
    route: string;
    push(url: string): void;
    replace(url: string): void;
    back(): void;
    forward(): void;
    reload(): void;
    prefetch(url: string): void;
    events: RouterEvents;
    isReady: boolean;
}
/**
 * Navigate to a new path via client-side routing.
 */
export declare function navigateTo(path: string): void;
/**
 * React hook that returns the current router instance.
 */
export declare function useRouter(): RexRouter;
export default useRouter;
//# sourceMappingURL=router.d.ts.map