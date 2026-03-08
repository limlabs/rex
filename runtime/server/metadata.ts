// Metadata API — app router head management
//
// Supports static `metadata` exports and async `generateMetadata` functions
// from layout and page files. Metadata is merged bottom-up (page overrides layout)
// with title template support.

interface MetadataTitle {
    default?: string;
    template?: string;
    absolute?: string;
}

interface MetadataOpenGraph {
    title?: string;
    description?: string;
    url?: string;
    siteName?: string;
    locale?: string;
    type?: string;
    images?: string | { url: string; width?: number; height?: number; alt?: string }[];
}

interface MetadataTwitter {
    card?: 'summary' | 'summary_large_image' | 'app' | 'player';
    site?: string;
    creator?: string;
    title?: string;
    description?: string;
    images?: string | string[];
}

interface MetadataIcons {
    icon?: string | { url: string; type?: string; sizes?: string }[];
    shortcut?: string;
    apple?: string | { url: string; sizes?: string }[];
}

interface MetadataAlternates {
    canonical?: string;
    languages?: Record<string, string>;
}

interface MetadataRobots {
    index?: boolean;
    follow?: boolean;
    googleBot?: string | { index?: boolean; follow?: boolean };
}

interface Metadata {
    title?: string | MetadataTitle;
    description?: string;
    keywords?: string | string[];
    openGraph?: MetadataOpenGraph;
    twitter?: MetadataTwitter;
    robots?: string | MetadataRobots;
    icons?: string | MetadataIcons;
    alternates?: MetadataAlternates;
    other?: Record<string, string>;
}

// Resolve the title from a metadata chain (layouts + page).
// Page title wins; layout templates wrap the page title.
// Template only applies when a child explicitly sets a title — the layout's
// own `default` title is used as-is when no child overrides it.
function resolveTitle(chain: Metadata[]): string | null {
    let resolved: string | null = null;
    let template: string | null = null;
    // Track the index where the template was defined so we only apply it
    // when a deeper entry explicitly sets a title.
    let templateSourceIndex = -1;
    let lastTitleSetIndex = -1;

    // Walk from root layout to page
    for (let i = 0; i < chain.length; i++) {
        const m = chain[i];
        if (!m.title) continue;

        if (typeof m.title === 'string') {
            resolved = m.title;
            lastTitleSetIndex = i;
        } else {
            if (m.title.absolute) {
                resolved = m.title.absolute;
                template = null; // absolute resets template
                lastTitleSetIndex = i;
            } else if (m.title.default) {
                resolved = m.title.default;
                lastTitleSetIndex = i;
            }
            if (m.title.template) {
                template = m.title.template;
                templateSourceIndex = i;
            }
        }
    }

    // Only apply the template if the title was explicitly set by a child
    // deeper in the chain than the template source.
    if (resolved && template && lastTitleSetIndex > templateSourceIndex) {
        resolved = template.replace('%s', resolved);
    }

    return resolved;
}

// Merge metadata from layouts + page. Later entries override earlier ones.
function mergeMetadata(chain: Metadata[]): Metadata {
    const merged: Metadata = {};

    for (let i = 0; i < chain.length; i++) {
        const m = chain[i];
        if (m.description !== undefined) merged.description = m.description;
        if (m.keywords !== undefined) merged.keywords = m.keywords;
        if (m.openGraph !== undefined) {
            merged.openGraph = merged.openGraph
                ? { ...merged.openGraph, ...m.openGraph }
                : m.openGraph;
        }
        if (m.twitter !== undefined) {
            merged.twitter = merged.twitter
                ? { ...merged.twitter, ...m.twitter }
                : m.twitter;
        }
        if (m.robots !== undefined) merged.robots = m.robots;
        if (m.icons !== undefined) merged.icons = m.icons;
        if (m.alternates !== undefined) {
            merged.alternates = merged.alternates
                ? { ...merged.alternates, ...m.alternates }
                : m.alternates;
        }
        if (m.other !== undefined) {
            merged.other = merged.other
                ? { ...merged.other, ...m.other }
                : m.other;
        }
    }

    return merged;
}

function escapeHtml(s: string): string {
    return s.replace(/&/g, '&amp;').replace(/"/g, '&quot;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

// Convert resolved metadata to an HTML string of head elements.
// Called from flight.ts (included in the same IIFE via include_str!)
// eslint-disable-next-line no-unused-vars
function metadataToHtml(chain: Metadata[]): string {
    const title = resolveTitle(chain);
    const merged = mergeMetadata(chain);
    const parts: string[] = [];

    // Title
    if (title) {
        parts.push(`<title>${escapeHtml(title)}</title>`);
    }

    // Description
    if (merged.description) {
        parts.push(`<meta name="description" content="${escapeHtml(merged.description)}" />`);
    }

    // Keywords
    if (merged.keywords) {
        const kw = Array.isArray(merged.keywords) ? merged.keywords.join(', ') : merged.keywords;
        parts.push(`<meta name="keywords" content="${escapeHtml(kw)}" />`);
    }

    // Robots
    if (merged.robots) {
        if (typeof merged.robots === 'string') {
            parts.push(`<meta name="robots" content="${escapeHtml(merged.robots)}" />`);
        } else {
            const directives: string[] = [];
            if (merged.robots.index === false) directives.push('noindex');
            else if (merged.robots.index === true) directives.push('index');
            if (merged.robots.follow === false) directives.push('nofollow');
            else if (merged.robots.follow === true) directives.push('follow');
            if (directives.length > 0) {
                parts.push(`<meta name="robots" content="${directives.join(', ')}" />`);
            }
            if (merged.robots.googleBot) {
                const gb = typeof merged.robots.googleBot === 'string'
                    ? merged.robots.googleBot
                    : [
                        merged.robots.googleBot.index === false ? 'noindex' : 'index',
                        merged.robots.googleBot.follow === false ? 'nofollow' : 'follow',
                      ].join(', ');
                parts.push(`<meta name="googlebot" content="${escapeHtml(gb)}" />`);
            }
        }
    }

    // Open Graph
    if (merged.openGraph) {
        const og = merged.openGraph;
        if (og.title) parts.push(`<meta property="og:title" content="${escapeHtml(og.title)}" />`);
        if (og.description) parts.push(`<meta property="og:description" content="${escapeHtml(og.description)}" />`);
        if (og.url) parts.push(`<meta property="og:url" content="${escapeHtml(og.url)}" />`);
        if (og.siteName) parts.push(`<meta property="og:site_name" content="${escapeHtml(og.siteName)}" />`);
        if (og.locale) parts.push(`<meta property="og:locale" content="${escapeHtml(og.locale)}" />`);
        if (og.type) parts.push(`<meta property="og:type" content="${escapeHtml(og.type)}" />`);
        if (og.images) {
            const images = typeof og.images === 'string' ? [{ url: og.images }] : og.images;
            for (let i = 0; i < images.length; i++) {
                const img = images[i];
                parts.push(`<meta property="og:image" content="${escapeHtml(img.url)}" />`);
                if (img.width) parts.push(`<meta property="og:image:width" content="${img.width}" />`);
                if (img.height) parts.push(`<meta property="og:image:height" content="${img.height}" />`);
                if (img.alt) parts.push(`<meta property="og:image:alt" content="${escapeHtml(img.alt)}" />`);
            }
        }
    }

    // Twitter
    if (merged.twitter) {
        const tw = merged.twitter;
        if (tw.card) parts.push(`<meta name="twitter:card" content="${escapeHtml(tw.card)}" />`);
        if (tw.site) parts.push(`<meta name="twitter:site" content="${escapeHtml(tw.site)}" />`);
        if (tw.creator) parts.push(`<meta name="twitter:creator" content="${escapeHtml(tw.creator)}" />`);
        if (tw.title) parts.push(`<meta name="twitter:title" content="${escapeHtml(tw.title)}" />`);
        if (tw.description) parts.push(`<meta name="twitter:description" content="${escapeHtml(tw.description)}" />`);
        if (tw.images) {
            const imgs = typeof tw.images === 'string' ? [tw.images] : tw.images;
            for (let i = 0; i < imgs.length; i++) {
                parts.push(`<meta name="twitter:image" content="${escapeHtml(imgs[i])}" />`);
            }
        }
    }

    // Icons
    if (merged.icons) {
        if (typeof merged.icons === 'string') {
            parts.push(`<link rel="icon" href="${escapeHtml(merged.icons)}" />`);
        } else {
            if (merged.icons.icon) {
                const icons = typeof merged.icons.icon === 'string'
                    ? [{ url: merged.icons.icon }]
                    : merged.icons.icon;
                for (let i = 0; i < icons.length; i++) {
                    let tag = `<link rel="icon" href="${escapeHtml(icons[i].url)}"`;
                    if (icons[i].type) tag += ` type="${escapeHtml(icons[i].type!)}"`;
                    if (icons[i].sizes) tag += ` sizes="${escapeHtml(icons[i].sizes!)}"`;
                    tag += ' />';
                    parts.push(tag);
                }
            }
            if (merged.icons.shortcut) {
                parts.push(`<link rel="shortcut icon" href="${escapeHtml(merged.icons.shortcut)}" />`);
            }
            if (merged.icons.apple) {
                const apples = typeof merged.icons.apple === 'string'
                    ? [{ url: merged.icons.apple }]
                    : merged.icons.apple;
                for (let i = 0; i < apples.length; i++) {
                    let tag = `<link rel="apple-touch-icon" href="${escapeHtml(apples[i].url)}"`;
                    if (apples[i].sizes) tag += ` sizes="${escapeHtml(apples[i].sizes!)}"`;
                    tag += ' />';
                    parts.push(tag);
                }
            }
        }
    }

    // Alternates
    if (merged.alternates) {
        if (merged.alternates.canonical) {
            parts.push(`<link rel="canonical" href="${escapeHtml(merged.alternates.canonical)}" />`);
        }
        if (merged.alternates.languages) {
            for (const [lang, url] of Object.entries(merged.alternates.languages)) {
                parts.push(`<link rel="alternate" hreflang="${escapeHtml(lang)}" href="${escapeHtml(url)}" />`);
            }
        }
    }

    // Other custom meta tags
    if (merged.other) {
        for (const [name, content] of Object.entries(merged.other)) {
            parts.push(`<meta name="${escapeHtml(name)}" content="${escapeHtml(content)}" />`);
        }
    }

    return parts.join('');
}
