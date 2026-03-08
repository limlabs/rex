/**
 * Redirect from a server action.
 * @param url - The URL to redirect to
 * @param status - HTTP status code (default: 303)
 */
export declare function redirect(url: string, status?: number): never;

/**
 * Return a 404 Not Found from a server action.
 */
export declare function notFound(): never;

/**
 * Access request cookies inside a server action.
 */
export declare function cookies(): Readonly<Record<string, string>>;

/**
 * Access request headers inside a server action.
 */
export declare function headers(): Readonly<Record<string, string>>;
