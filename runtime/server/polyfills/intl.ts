// Intl polyfills for bare V8 with small-icu.
// V8 ships with small-icu which lacks Intl.supportedValuesOf and some
// locale-sensitive operations. This polyfill adds enough to satisfy
// PayloadCMS and similar frameworks that use timezone/locale validation.

/* eslint-disable @typescript-eslint/no-explicit-any */

// ---------- Intl.supportedValuesOf ----------
// V8's small-icu may have this method but it throws RangeError due to
// missing ICU data. Override unconditionally with a working implementation.
if (typeof Intl !== 'undefined') {
    (Intl as any).supportedValuesOf = function supportedValuesOf(key: string): string[] {
        if (key === 'timeZone') {
            return [
                'Africa/Abidjan', 'Africa/Cairo', 'Africa/Casablanca', 'Africa/Johannesburg',
                'Africa/Lagos', 'Africa/Nairobi',
                'America/Anchorage', 'America/Argentina/Buenos_Aires', 'America/Bogota',
                'America/Buenos_Aires', 'America/Caracas', 'America/Chicago', 'America/Denver',
                'America/Guatemala', 'America/Halifax', 'America/Lima', 'America/Los_Angeles',
                'America/Mexico_City', 'America/New_York', 'America/Phoenix', 'America/Santiago',
                'America/Sao_Paulo', 'America/St_Johns', 'America/Tijuana',
                'America/Toronto', 'America/Vancouver',
                'Asia/Almaty', 'Asia/Baghdad', 'Asia/Baku', 'Asia/Bangkok', 'Asia/Calcutta',
                'Asia/Colombo', 'Asia/Dhaka', 'Asia/Dubai', 'Asia/Hong_Kong', 'Asia/Istanbul',
                'Asia/Jakarta', 'Asia/Jerusalem', 'Asia/Karachi', 'Asia/Kathmandu',
                'Asia/Kolkata', 'Asia/Kuala_Lumpur', 'Asia/Manila', 'Asia/Riyadh', 'Asia/Seoul',
                'Asia/Shanghai', 'Asia/Singapore', 'Asia/Taipei', 'Asia/Tashkent',
                'Asia/Tehran', 'Asia/Tokyo',
                'Atlantic/Azores', 'Atlantic/Cape_Verde', 'Atlantic/Reykjavik',
                'Atlantic/South_Georgia',
                'Australia/Adelaide', 'Australia/Brisbane', 'Australia/Darwin',
                'Australia/Hobart', 'Australia/Melbourne', 'Australia/Perth', 'Australia/Sydney',
                'Europe/Amsterdam', 'Europe/Athens', 'Europe/Belgrade', 'Europe/Berlin',
                'Europe/Brussels', 'Europe/Bucharest', 'Europe/Budapest', 'Europe/Copenhagen',
                'Europe/Dublin', 'Europe/Helsinki', 'Europe/Kiev', 'Europe/Lisbon',
                'Europe/London', 'Europe/Madrid', 'Europe/Moscow', 'Europe/Oslo',
                'Europe/Paris', 'Europe/Prague', 'Europe/Rome', 'Europe/Stockholm',
                'Europe/Vienna', 'Europe/Warsaw', 'Europe/Zurich',
                'Pacific/Auckland', 'Pacific/Fiji', 'Pacific/Gambier', 'Pacific/Guam',
                'Pacific/Honolulu', 'Pacific/Midway', 'Pacific/Niue', 'Pacific/Noumea',
                'Pacific/Rarotonga',
                'US/Alaska', 'US/Central', 'US/Eastern', 'US/Hawaii', 'US/Mountain', 'US/Pacific',
                'UTC',
            ];
        }
        if (key === 'calendar') {
            return ['buddhist', 'chinese', 'coptic', 'ethiopic', 'gregory', 'hebrew',
                    'indian', 'islamic', 'iso8601', 'japanese', 'persian', 'roc'];
        }
        if (key === 'collation') {
            return ['default', 'ducet', 'emoji', 'eor'];
        }
        if (key === 'currency') {
            return ['AUD', 'BRL', 'CAD', 'CHF', 'CNY', 'EUR', 'GBP', 'HKD', 'INR', 'JPY',
                    'KRW', 'MXN', 'NOK', 'NZD', 'RUB', 'SEK', 'SGD', 'TRY', 'USD', 'ZAR'];
        }
        if (key === 'numberingSystem') {
            return ['arab', 'arabext', 'bali', 'beng', 'deva', 'fullwide', 'gujr', 'guru',
                    'hanidec', 'khmr', 'knda', 'laoo', 'latn', 'limb', 'mlym', 'mong', 'mymr',
                    'orya', 'tamldec', 'telu', 'thai', 'tibt'];
        }
        if (key === 'unit') {
            return ['acre', 'bit', 'byte', 'celsius', 'centimeter', 'day', 'degree',
                    'fahrenheit', 'foot', 'gallon', 'gigabit', 'gigabyte', 'gram', 'hectare',
                    'hour', 'inch', 'kilobit', 'kilobyte', 'kilogram', 'kilometer',
                    'liter', 'megabit', 'megabyte', 'meter', 'mile', 'milliliter',
                    'millimeter', 'millisecond', 'minute', 'month', 'ounce', 'percent',
                    'petabyte', 'pound', 'second', 'stone', 'terabit', 'terabyte',
                    'week', 'yard', 'year'];
        }
        throw new RangeError(`Invalid key: ${key}`);
    };
}

// ---------- Intl.DateTimeFormat fallback ----------
// Wrap Intl.DateTimeFormat to catch ICU errors and provide basic formatting.
if (typeof Intl !== 'undefined' && typeof Intl.DateTimeFormat === 'function') {
    const OriginalDateTimeFormat = Intl.DateTimeFormat;
    (Intl as any).DateTimeFormat = function DateTimeFormat(
        locales?: string | string[],
        options?: any,
    ): any {
        try {
            return new OriginalDateTimeFormat(locales, options);
        } catch {
            // Fallback: return a formatter that uses Date.toISOString()
            return {
                format(date: Date) {
                    if (date instanceof Date && !isNaN(date.getTime())) {
                        return date.toISOString();
                    }
                    return String(date);
                },
                resolvedOptions() {
                    return {
                        locale: typeof locales === 'string' ? locales : 'en-US',
                        timeZone: options?.timeZone || 'UTC',
                        calendar: 'gregory',
                        numberingSystem: 'latn',
                    };
                },
                formatToParts(date: Date) {
                    const s = date instanceof Date ? date.toISOString() : String(date);
                    return [{ type: 'literal', value: s }];
                },
            };
        }
    };
    // Preserve static methods
    (Intl as any).DateTimeFormat.supportedLocalesOf =
        OriginalDateTimeFormat.supportedLocalesOf?.bind(OriginalDateTimeFormat) ||
        (() => ['en-US']);
}

// ---------- Intl.RelativeTimeFormat ----------
// V8 small-icu may not have this at all.
if (typeof Intl !== 'undefined' && typeof (Intl as any).RelativeTimeFormat !== 'function') {
    (Intl as any).RelativeTimeFormat = class RelativeTimeFormat {
        private _locale: string;
        private _options: any;
        constructor(locales?: string | string[], options?: any) {
            this._locale = typeof locales === 'string' ? locales : 'en';
            this._options = options || {};
        }
        format(value: number, unit: string): string {
            const abs = Math.abs(value);
            const plural = abs === 1 ? '' : 's';
            if (value < 0) return `${abs} ${unit}${plural} ago`;
            if (value > 0) return `in ${abs} ${unit}${plural}`;
            return `now`;
        }
        resolvedOptions() {
            return { locale: this._locale, ...this._options };
        }
    };
}
