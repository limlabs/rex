// RSC Flight Protocol — Simplified Implementation for Rex
//
// Serializes a React element tree into Rex flight data format.
// Server components are called (rendered) during serialization.
// Client references are emitted as markers with their ref IDs.
//
// Flight format: newline-delimited rows
//   J:<id>:<json>     — JSON model node (element tree fragment)
//   M:<id>:<json>     — Client module reference
//   E:<id>:<json>     — Error
//
// References between rows use "$<id>" strings as placeholders.

var _nextId = 0;

function _resetIds() {
    _nextId = 0;
}

function _allocId() {
    return _nextId++;
}

// Serialize a value for the flight wire format.
// Returns [rows, rootReference] where rootReference is "$<id>" or a literal.
function _serialize(value, rows) {
    // Primitives
    if (value === null || value === undefined) return null;
    if (typeof value === 'string') return value;
    if (typeof value === 'number') return value;
    if (typeof value === 'boolean') return value;

    // Arrays
    if (Array.isArray(value)) {
        return value.map(function(item) { return _serialize(item, rows); });
    }

    // React element
    var reactElementSymbol = Symbol.for('react.element');
    var reactTransitionalSymbol = Symbol.for('react.transitional.element');

    if (value && (value.$$typeof === reactElementSymbol || value.$$typeof === reactTransitionalSymbol)) {
        var type = value.type;
        var props = value.props;

        // Client reference — emit module row
        if (type && type.$$typeof === Symbol.for('react.client.reference')) {
            var modId = _allocId();
            var modRow = {
                id: type.$$id,
                name: type.$$name
            };
            rows.push('M:' + modId + ':' + JSON.stringify(modRow));

            // Serialize the props (children may contain more elements)
            var serializedProps = _serializeProps(props, rows);
            var elemId = _allocId();
            rows.push('J:' + elemId + ':' + JSON.stringify({
                t: '$M' + modId,
                p: serializedProps
            }));
            return '$' + elemId;
        }

        // Server component (function) — call it to render
        if (typeof type === 'function') {
            var rendered;
            try {
                rendered = type(props);
            } catch (err) {
                var errId = _allocId();
                rows.push('E:' + errId + ':' + JSON.stringify({
                    message: String(err),
                    stack: err && err.stack ? err.stack : ''
                }));
                return '$E' + errId;
            }
            return _serialize(rendered, rows);
        }

        // Fragment (type is undefined or Symbol.for('react.fragment'))
        if (type === undefined || type === Symbol.for('react.fragment')) {
            return _serialize(props.children, rows);
        }

        // HTML element (string tag like "div", "h1")
        if (typeof type === 'string') {
            var serializedProps2 = _serializeProps(props, rows);
            var elemId2 = _allocId();
            rows.push('J:' + elemId2 + ':' + JSON.stringify({
                t: type,
                p: serializedProps2
            }));
            return '$' + elemId2;
        }

        // Unknown type — skip
        return null;
    }

    // Plain object (e.g., props that are objects)
    if (typeof value === 'object') {
        var result = {};
        for (var key in value) {
            if (Object.prototype.hasOwnProperty.call(value, key)) {
                result[key] = _serialize(value[key], rows);
            }
        }
        return result;
    }

    // Functions and other non-serializable types
    return null;
}

function _serializeProps(props, rows) {
    if (!props) return {};
    var result = {};
    for (var key in props) {
        if (!Object.prototype.hasOwnProperty.call(props, key)) continue;
        if (key === 'ref') continue; // Skip refs
        result[key] = _serialize(props[key], rows);
    }
    return result;
}

// Main entry: render a React element tree to flight data string.
// Returns the flight data as a newline-delimited string.
globalThis.__rex_render_flight = function(routeKey, propsJson) {
    var props = JSON.parse(propsJson);
    var Page = globalThis.__rex_app_pages[routeKey];
    if (!Page) {
        return 'E:0:' + JSON.stringify({ message: 'Page not found: ' + routeKey, stack: '' });
    }

    var layouts = globalThis.__rex_app_layout_chains[routeKey] || [];

    // Build nested layout tree: Layout1(Layout2(Page))
    var element = __rex_createElement(Page, props);
    for (var i = layouts.length - 1; i >= 0; i--) {
        element = __rex_createElement(layouts[i], { children: element });
    }

    _resetIds();
    var rows = [];
    var rootRef = _serialize(element, rows);

    // Add root row pointing to the root reference
    var rootId = _allocId();
    rows.push('J:' + rootId + ':' + JSON.stringify(rootRef));
    rows.push('R:' + rootId); // Root marker

    return rows.join('\n');
};

// Two-pass render: produce flight data, then render to HTML.
// For initial page loads, we need both the flight data (for hydration)
// and the HTML (for immediate display).
globalThis.__rex_render_rsc_to_html = function(routeKey, propsJson) {
    var props = JSON.parse(propsJson);
    var Page = globalThis.__rex_app_pages[routeKey];
    if (!Page) {
        return JSON.stringify({
            body: '<div>Page not found</div>',
            head: '',
            flight: 'E:0:' + JSON.stringify({ message: 'Page not found: ' + routeKey })
        });
    }

    var layouts = globalThis.__rex_app_layout_chains[routeKey] || [];

    // Build element tree
    var element = __rex_createElement(Page, props);
    for (var i = layouts.length - 1; i >= 0; i--) {
        element = __rex_createElement(layouts[i], { children: element });
    }

    // Pass 1: Generate flight data
    _resetIds();
    var rows = [];
    var rootRef = _serialize(element, rows);
    var rootId = _allocId();
    rows.push('J:' + rootId + ':' + JSON.stringify(rootRef));
    rows.push('R:' + rootId);
    var flightData = rows.join('\n');

    // Pass 2: Render to HTML (re-create the element since server components
    // are functions that may have side effects)
    var element2 = __rex_createElement(Page, props);
    for (var i = layouts.length - 1; i >= 0; i--) {
        element2 = __rex_createElement(layouts[i], { children: element2 });
    }

    try {
        var html = __rex_renderToString(element2);
        return JSON.stringify({
            body: html,
            head: '',
            flight: flightData
        });
    } catch (e) {
        return JSON.stringify({
            body: '<div>Render error: ' + String(e) + '</div>',
            head: '',
            flight: flightData
        });
    }
};
