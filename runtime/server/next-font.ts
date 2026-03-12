// next/font/google and next/font/local stubs for Rex server bundles.
// Returns a font object with className and style for SSR.

/* eslint-disable @typescript-eslint/no-explicit-any */

function createFont(_options: any): any {
    return {
        className: '',
        style: { fontFamily: '' },
        variable: '',
    };
}

// Named exports for common Google fonts
export const Inter = createFont;
export const Roboto = createFont;
export const Poppins = createFont;
export const Montserrat = createFont;
export const Open_Sans = createFont;
export const Lato = createFont;

// Default export is a function factory (for next/font/local)
export default createFont;
