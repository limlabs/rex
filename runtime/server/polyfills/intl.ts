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

// ---------- Intl.DateTimeFormat pure-JS replacement ----------
// V8 small-icu lacks locale data for most locales. When ICU's
// DateTimePatternGenerator::createInstance() fails, V8 calls
// FatalProcessOutOfMemory at the C++ level — this bypasses JS try/catch
// entirely and crashes the process. We must avoid calling the native
// constructor for any locale/options combo that might not be supported.
//
// Strategy: replace Intl.DateTimeFormat entirely with a pure-JS
// implementation that provides reasonable en-US formatting without ICU.
if (typeof Intl !== 'undefined') {
    const MONTH_NAMES = [
        'January', 'February', 'March', 'April', 'May', 'June',
        'July', 'August', 'September', 'October', 'November', 'December',
    ];
    const MONTH_SHORT = [
        'Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun',
        'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec',
    ];
    const DAY_NAMES = [
        'Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday',
    ];
    const DAY_SHORT = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];

    // Timezone offset database (UTC offset in minutes) for common IANA zones.
    // These are approximate (no DST) but sufficient for server-side rendering.
    const TZ_OFFSETS: Record<string, number> = {
        'UTC': 0, 'GMT': 0,
        'US/Eastern': -300, 'America/New_York': -300,
        'US/Central': -360, 'America/Chicago': -360,
        'US/Mountain': -420, 'America/Denver': -420,
        'US/Pacific': -480, 'America/Los_Angeles': -480,
        'US/Alaska': -540, 'America/Anchorage': -540,
        'US/Hawaii': -600, 'Pacific/Honolulu': -600,
        'Europe/London': 0, 'Europe/Paris': 60, 'Europe/Berlin': 60,
        'Europe/Moscow': 180, 'Asia/Dubai': 240, 'Asia/Kolkata': 330,
        'Asia/Calcutta': 330, 'Asia/Shanghai': 480, 'Asia/Tokyo': 540,
        'Australia/Sydney': 660, 'Pacific/Auckland': 780,
        'America/Sao_Paulo': -180, 'America/Toronto': -300,
        'America/Vancouver': -480, 'America/Mexico_City': -360,
        'Asia/Singapore': 480, 'Asia/Hong_Kong': 480,
        'Asia/Seoul': 540, 'Asia/Bangkok': 420, 'Asia/Jakarta': 420,
        'Europe/Amsterdam': 60, 'Europe/Rome': 60, 'Europe/Madrid': 60,
        'Europe/Stockholm': 60, 'Europe/Zurich': 60, 'Europe/Vienna': 60,
        'Europe/Warsaw': 60, 'Europe/Prague': 60, 'Europe/Athens': 120,
        'Europe/Helsinki': 120, 'Europe/Istanbul': 180,
        'Europe/Lisbon': 0, 'Europe/Dublin': 0,
        'Asia/Karachi': 300, 'Asia/Dhaka': 360, 'Asia/Taipei': 480,
        'Asia/Manila': 480, 'Asia/Riyadh': 180, 'Asia/Tehran': 210,
        'Asia/Baghdad': 180, 'Asia/Jerusalem': 120,
        'Africa/Cairo': 120, 'Africa/Lagos': 60, 'Africa/Nairobi': 180,
        'Africa/Johannesburg': 120, 'Africa/Casablanca': 60,
        'Australia/Melbourne': 660, 'Australia/Perth': 480,
        'Australia/Brisbane': 600, 'Australia/Adelaide': 570,
        'Pacific/Fiji': 720, 'America/Santiago': -240,
        'America/Lima': -300, 'America/Bogota': -300,
        'America/Halifax': -240, 'America/Phoenix': -420,
        'Asia/Almaty': 360, 'Asia/Tashkent': 300, 'Asia/Colombo': 330,
        'Asia/Kuala_Lumpur': 480, 'Asia/Baku': 240,
    };

    function applyTzOffset(date: Date, tz?: string): Date {
        if (!tz || tz === 'UTC' || tz === 'GMT') return date;
        const offset = TZ_OFFSETS[tz];
        if (offset === undefined) return date; // unknown tz, use UTC
        // Convert UTC date to the target timezone
        const utcMs = date.getTime() + date.getTimezoneOffset() * 60000;
        return new Date(utcMs + offset * 60000);
    }

    function pad2(n: number): string {
        return n < 10 ? '0' + n : '' + n;
    }

    function formatGmtOffset(offsetMin: number): string {
        const sign = offsetMin >= 0 ? '+' : '-';
        const abs = Math.abs(offsetMin);
        const h = Math.floor(abs / 60);
        const m = abs % 60;
        return `GMT${sign}${pad2(h)}:${pad2(m)}`;
    }

    class DateTimeFormatPolyfill {
        private _locale: string;
        private _options: any;

        constructor(locales?: string | string[], options?: any) {
            this._locale = typeof locales === 'string' ? locales
                         : Array.isArray(locales) && locales.length > 0 ? locales[0]
                         : 'en-US';
            this._options = options || {};
        }

        format(date?: Date | number): string {
            const d = date instanceof Date ? date
                    : typeof date === 'number' ? new Date(date)
                    : new Date();
            if (isNaN(d.getTime())) return 'Invalid Date';
            const adj = applyTzOffset(d, this._options.timeZone);
            return this._formatDate(adj);
        }

        private _formatDate(d: Date): string {
            const opts = this._options;
            const parts: string[] = [];

            // timeZoneName-only format (used by PayloadCMS for UTC offset extraction)
            if (opts.timeZoneName && !opts.year && !opts.month && !opts.day &&
                !opts.weekday && !opts.era) {
                if (opts.hour) {
                    parts.push(this._formatHour(d));
                }
                const offset = TZ_OFFSETS[opts.timeZone] ?? 0;
                const tzName = opts.timeZoneName;
                if (tzName === 'longOffset' || tzName === 'shortOffset') {
                    parts.push(formatGmtOffset(offset));
                } else if (tzName === 'long' || tzName === 'short') {
                    parts.push(opts.timeZone || 'UTC');
                } else {
                    parts.push(formatGmtOffset(offset));
                }
                return parts.join(' ');
            }

            // Date parts
            if (opts.weekday) {
                const dayIdx = d.getUTCDay();
                parts.push(opts.weekday === 'long' ? DAY_NAMES[dayIdx]
                         : opts.weekday === 'short' ? DAY_SHORT[dayIdx]
                         : DAY_SHORT[dayIdx]);
            }
            if (opts.month && !opts.day && !opts.weekday) {
                // Month + year only
                const monthIdx = d.getUTCMonth();
                const monthStr = opts.month === 'long' ? MONTH_NAMES[monthIdx]
                               : opts.month === 'short' ? MONTH_SHORT[monthIdx]
                               : opts.month === 'narrow' ? MONTH_NAMES[monthIdx][0]
                               : opts.month === '2-digit' ? pad2(monthIdx + 1)
                               : '' + (monthIdx + 1);
                if (opts.year) {
                    const yr = opts.year === '2-digit'
                        ? '' + (d.getUTCFullYear() % 100)
                        : '' + d.getUTCFullYear();
                    parts.push(`${monthStr} ${yr}`);
                } else {
                    parts.push(monthStr);
                }
            } else if (opts.year || opts.month || opts.day) {
                const yr = d.getUTCFullYear();
                const mo = d.getUTCMonth() + 1;
                const dy = d.getUTCDate();
                if (opts.year && !opts.month && !opts.day) {
                    parts.push(opts.year === '2-digit' ? '' + (yr % 100) : '' + yr);
                } else {
                    const monthIdx = d.getUTCMonth();
                    const monthStr = opts.month === 'long' ? MONTH_NAMES[monthIdx]
                                   : opts.month === 'short' ? MONTH_SHORT[monthIdx]
                                   : opts.month === '2-digit' ? pad2(mo)
                                   : '' + mo;
                    if (opts.month === 'long' || opts.month === 'short') {
                        parts.push(`${monthStr} ${dy}, ${yr}`);
                    } else {
                        parts.push(`${pad2(mo)}/${pad2(dy)}/${yr}`);
                    }
                }
            }

            // Time parts
            if (opts.hour || opts.minute || opts.second) {
                parts.push(this._formatTime(d));
            }

            if (parts.length === 0) {
                // Default: MM/DD/YYYY
                const mo = d.getUTCMonth() + 1;
                const dy = d.getUTCDate();
                const yr = d.getUTCFullYear();
                return `${pad2(mo)}/${pad2(dy)}/${yr}`;
            }

            return parts.join(', ');
        }

        private _formatHour(d: Date): string {
            let h = d.getUTCHours();
            const hourCycle = this._options.hourCycle || (this._options.hour12 === false ? 'h23' : 'h12');
            if (hourCycle === 'h12' || hourCycle === 'h11') {
                const ampm = h >= 12 ? 'PM' : 'AM';
                h = h % 12 || 12;
                return `${h} ${ampm}`;
            }
            return pad2(h);
        }

        private _formatTime(d: Date): string {
            const parts: string[] = [];
            const h = d.getUTCHours();
            const m = d.getUTCMinutes();
            const s = d.getUTCSeconds();
            const hourCycle = this._options.hourCycle || (this._options.hour12 === false ? 'h23' : 'h12');
            let ampm = '';

            if (this._options.hour) {
                if (hourCycle === 'h12' || hourCycle === 'h11') {
                    ampm = h >= 12 ? ' PM' : ' AM';
                    const h12 = h % 12 || 12;
                    parts.push(this._options.hour === '2-digit' ? pad2(h12) : '' + h12);
                } else {
                    parts.push(pad2(h));
                }
            }
            if (this._options.minute) {
                parts.push(pad2(m));
            }
            if (this._options.second) {
                parts.push(pad2(s));
            }
            return parts.join(':') + ampm;
        }

        formatToParts(date?: Date | number): Array<{type: string, value: string}> {
            const formatted = this.format(date);
            return [{ type: 'literal', value: formatted }];
        }

        resolvedOptions(): any {
            return {
                locale: this._locale,
                timeZone: this._options.timeZone || 'UTC',
                calendar: 'gregory',
                numberingSystem: 'latn',
                year: this._options.year,
                month: this._options.month,
                day: this._options.day,
                hour: this._options.hour,
                minute: this._options.minute,
                second: this._options.second,
                hourCycle: this._options.hourCycle || 'h12',
            };
        }
    }

    (Intl as any).DateTimeFormat = DateTimeFormatPolyfill;
    (Intl as any).DateTimeFormat.supportedLocalesOf = () => ['en-US'];
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
