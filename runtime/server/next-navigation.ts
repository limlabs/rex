// next/navigation stubs for Rex server bundles.

/* eslint-disable @typescript-eslint/no-explicit-any */

export function useRouter(): any {
    return {
        push(_href: string) {},
        replace(_href: string) {},
        refresh() {},
        back() {},
        forward() {},
        prefetch(_href: string) {},
    };
}

export function usePathname(): string { return '/'; }
export function useSearchParams(): any { return new URLSearchParams(); }
export function useParams(): any { return {}; }
export function useSelectedLayoutSegment(): string | null { return null; }
export function useSelectedLayoutSegments(): string[] { return []; }

export function notFound(): never {
    throw Object.assign(new Error('NEXT_NOT_FOUND'), { digest: 'NEXT_NOT_FOUND' });
}

export function redirect(url: string): never {
    throw Object.assign(new Error('NEXT_REDIRECT'), { digest: 'NEXT_REDIRECT', url });
}

export function permanentRedirect(url: string): never {
    throw Object.assign(new Error('NEXT_REDIRECT'), { digest: 'NEXT_REDIRECT', url, permanent: true });
}

const navigation = {
    useRouter, usePathname, useSearchParams, useParams,
    useSelectedLayoutSegment, useSelectedLayoutSegments,
    notFound, redirect, permanentRedirect,
};
export default navigation;
