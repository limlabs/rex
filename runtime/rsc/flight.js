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
var _pendingSlots = [];
var _hasPending = false;

function _resetIds() {
    _nextId = 0;
}

function _resetPending() {
    _pendingSlots = [];
    _hasPending = false;
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
            // Async server component — returns a Promise
            if (rendered && typeof rendered.then === 'function') {
                var slotId = _allocId();
                var slot = { id: slotId, promise: rendered, resolved: false, rejected: false, value: undefined, error: undefined };
                rendered.then(
                    function(v) { slot.resolved = true; slot.value = v; },
                    function(e) { slot.rejected = true; slot.error = e; }
                );
                _pendingSlots.push(slot);
                _hasPending = true;
                rows.push('J:' + slotId + ':null');  // placeholder
                return '$' + slotId;
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

// Resolve pending async slots. Called iteratively from Rust after
// fetch loop + microtask checkpoint. Returns "done" or "pending".
globalThis.__rex_resolve_rsc_pending = function() {
    var stillPending = false;
    var rscRows = globalThis.__rex_rsc_rows;

    // Build index map: placeholder string → row index for O(1) lookup.
    // Placeholders are exactly "J:<id>:null" — we match by checking the
    // substring after the second colon is literally "null" (not JSON ending in :null).
    var rowIndexMap = {};
    for (var ri = 0; ri < rscRows.length; ri++) {
        var row = rscRows[ri];
        if (row.indexOf('J:') === 0) {
            var secondColon = row.indexOf(':', 2);
            if (secondColon !== -1 && row.substring(secondColon + 1) === 'null') {
                rowIndexMap[row] = ri;
            }
        }
    }

    for (var i = 0; i < _pendingSlots.length; i++) {
        var slot = _pendingSlots[i];
        if (slot.resolved) {
            // Re-serialize the resolved value, replacing the placeholder row
            var newRows = [];
            var refValue = _serialize(slot.value, newRows);
            // Find and replace the placeholder row for this slot
            var placeholder = 'J:' + slot.id + ':null';
            var j = rowIndexMap[placeholder];
            if (j !== undefined) {
                // Replace with the resolved rows + a final row for this slot
                rscRows.splice(j, 1);
                // Insert new rows at the same position
                for (var k = 0; k < newRows.length; k++) {
                    rscRows.splice(j + k, 0, newRows[k]);
                }
                // Add the resolved reference row
                rscRows.splice(j + newRows.length, 0, 'J:' + slot.id + ':' + JSON.stringify(refValue));

                // Rebuild index map after splice (indices shifted)
                rowIndexMap = {};
                for (var ri2 = 0; ri2 < rscRows.length; ri2++) {
                    var row2 = rscRows[ri2];
                    if (row2.indexOf('J:') === 0) {
                        var sc2 = row2.indexOf(':', 2);
                        if (sc2 !== -1 && row2.substring(sc2 + 1) === 'null') {
                            rowIndexMap[row2] = ri2;
                        }
                    }
                }
            }
            // Mark as fully handled
            slot.resolved = false;
            slot.value = undefined;
            slot.promise = null;
        } else if (slot.rejected) {
            var placeholder2 = 'J:' + slot.id + ':null';
            var j2 = rowIndexMap[placeholder2];
            if (j2 !== undefined) {
                rscRows[j2] = 'E:' + slot.id + ':' + JSON.stringify({
                    message: String(slot.error),
                    stack: slot.error && slot.error.stack ? slot.error.stack : ''
                });
                delete rowIndexMap[placeholder2];
            }
            slot.rejected = false;
            slot.error = undefined;
            slot.promise = null;
        }
    }
    // Remove fully-handled slots and check if any are still active.
    // This prevents unbounded accumulation of inert slots in deeply nested async trees.
    var activeSlots = [];
    for (var n = 0; n < _pendingSlots.length; n++) {
        var s = _pendingSlots[n];
        if (s.promise !== null) {
            activeSlots.push(s);
            stillPending = true;
        }
    }
    _pendingSlots = activeSlots;
    _hasPending = stillPending;
    return stillPending ? 'pending' : 'done';
};

// Finalize flight data after all async slots are resolved.
globalThis.__rex_finalize_rsc_flight = function() {
    var result = globalThis.__rex_rsc_rows.join('\n');
    globalThis.__rex_rsc_rows = null;
    _resetPending();
    return result;
};

// Convert resolved flight rows to HTML string.
// Parses flight data and reconstructs HTML without calling React renderToString
// (which can't handle async components).
function _flightToHtml(rows) {
    var nodes = {};
    var rootId = null;

    for (var i = 0; i < rows.length; i++) {
        var row = rows[i];
        if (row.indexOf('J:') === 0) {
            var firstColon = row.indexOf(':');
            var secondColon = row.indexOf(':', firstColon + 1);
            var id = row.substring(firstColon + 1, secondColon);
            var json = row.substring(secondColon + 1);
            try {
                nodes[id] = JSON.parse(json);
            } catch (e) {
                nodes[id] = null;
            }
        } else if (row.indexOf('M:') === 0) {
            // Client module reference — store for lookup
            var firstColon2 = row.indexOf(':');
            var secondColon2 = row.indexOf(':', firstColon2 + 1);
            var modId = row.substring(firstColon2 + 1, secondColon2);
            var modJson = row.substring(secondColon2 + 1);
            try {
                nodes['M' + modId] = JSON.parse(modJson);
            } catch (e) {}
        } else if (row.indexOf('R:') === 0) {
            rootId = row.substring(2);
        }
    }

    if (rootId === null) {
        for (var ei = 0; ei < rows.length; ei++) {
            if (rows[ei].indexOf('E:') === 0) {
                var colonPos = rows[ei].indexOf(':', 2);
                var errJson = rows[ei].substring(colonPos + 1);
                try {
                    var err = JSON.parse(errJson);
                    return '<div style="color:red;font-family:monospace;padding:20px">RSC Error: ' +
                        (err.message || 'Unknown error').replace(/</g, '&lt;') + '</div>';
                } catch (e) {}
            }
        }
        return '';
    }
    return _renderFlightNode(nodes[rootId], nodes);
}

// Void elements that must not have a closing tag
var _voidElements = {area:1,base:1,br:1,col:1,embed:1,hr:1,img:1,input:1,link:1,meta:1,source:1,track:1,wbr:1};

function _renderFlightNode(value, nodes) {
    if (value === null || value === undefined) return '';
    if (typeof value === 'string') {
        // Check if it's a reference like "$5"
        if (value.charAt(0) === '$') {
            var refPart = value.substring(1);
            // Client module reference "$M<id>"
            if (refPart.charAt(0) === 'M') {
                return '<div data-client-component="true"></div>';
            }
            // Error reference "$E<id>"
            if (refPart.charAt(0) === 'E') return '';
            // Node reference
            if (nodes[refPart] !== undefined) {
                return _renderFlightNode(nodes[refPart], nodes);
            }
            return '';
        }
        // Escape HTML entities
        return value.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
    }
    if (typeof value === 'number' || typeof value === 'boolean') return String(value);

    if (Array.isArray(value)) {
        var out = '';
        for (var i = 0; i < value.length; i++) {
            out += _renderFlightNode(value[i], nodes);
        }
        return out;
    }

    if (typeof value === 'object') {
        // Element node: {"t":"div","p":{...}}
        if (value.t !== undefined) {
            var tag = value.t;
            // Client component reference: "$M<id>"
            if (typeof tag === 'string' && tag.charAt(0) === '$' && tag.charAt(1) === 'M') {
                var clientModId = tag.substring(2);
                var refId = nodes['M' + clientModId] ? nodes['M' + clientModId].id : '';
                return '<div data-client-component="' + _escapeAttr(String(refId)) + '"></div>';
            }
            // Sanitize tag name: only allow alphanumeric and hyphens
            if (typeof tag !== 'string' || !/^[a-zA-Z][a-zA-Z0-9-]*$/.test(tag)) {
                return '';
            }
            var attrs = '';
            var childrenHtml = '';
            var props = value.p || {};
            for (var key in props) {
                if (!Object.prototype.hasOwnProperty.call(props, key)) continue;
                if (key === 'children') {
                    childrenHtml = _renderFlightNode(props[key], nodes);
                } else if (key === 'className') {
                    attrs += ' class="' + _escapeAttr(String(props[key])) + '"';
                } else if (key === 'htmlFor') {
                    attrs += ' for="' + _escapeAttr(String(props[key])) + '"';
                } else if (!/^[a-zA-Z_][a-zA-Z0-9_-]*$/.test(key)) {
                    // Skip attribute names that could inject HTML
                    continue;
                } else if (typeof props[key] === 'string') {
                    attrs += ' ' + key + '="' + _escapeAttr(props[key]) + '"';
                } else if (typeof props[key] === 'number') {
                    attrs += ' ' + key + '="' + props[key] + '"';
                } else if (props[key] === true) {
                    attrs += ' ' + key;
                }
            }
            if (_voidElements[tag]) {
                return '<' + tag + attrs + '/>';
            }
            return '<' + tag + attrs + '>' + childrenHtml + '</' + tag + '>';
        }
        // Plain object — try to render its values
        return '';
    }

    return '';
}

function _escapeAttr(s) {
    return s.replace(/&/g, '&amp;').replace(/"/g, '&quot;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

// Finalize RSC-to-HTML after all async slots are resolved.
// Converts flight data to HTML directly (does not re-invoke React renderToString,
// which cannot handle async server components).
globalThis.__rex_finalize_rsc_to_html = function() {
    var rows = globalThis.__rex_rsc_rows;
    var flightData = rows.join('\n');
    globalThis.__rex_rsc_rows = null;
    _resetPending();

    var html = _flightToHtml(rows);
    return JSON.stringify({
        body: html,
        head: '',
        flight: flightData
    });
};

// Main entry: render a React element tree to flight data string.
// Returns the flight data as a newline-delimited string.
// Returns "__REX_RSC_ASYNC__" if async server components are pending.
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
    _resetPending();
    var rows = [];
    var rootRef = _serialize(element, rows);

    // Add root row pointing to the root reference
    var rootId = _allocId();
    rows.push('J:' + rootId + ':' + JSON.stringify(rootRef));
    rows.push('R:' + rootId); // Root marker

    if (_hasPending) {
        globalThis.__rex_rsc_rows = rows;
        return '__REX_RSC_ASYNC__';
    }

    return rows.join('\n');
};

// Two-pass render: produce flight data, then render to HTML.
// For initial page loads, we need both the flight data (for hydration)
// and the HTML (for immediate display).
// Returns "__REX_RSC_HTML_ASYNC__" if async server components are pending.
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
    _resetPending();
    var rows = [];
    var rootRef = _serialize(element, rows);
    var rootId = _allocId();
    rows.push('J:' + rootId + ':' + JSON.stringify(rootRef));
    rows.push('R:' + rootId);

    if (_hasPending) {
        globalThis.__rex_rsc_rows = rows;
        return '__REX_RSC_HTML_ASYNC__';
    }

    var flightData = rows.join('\n');
    var html = _flightToHtml(rows);

    return JSON.stringify({
        body: html,
        head: '',
        flight: flightData
    });
};
