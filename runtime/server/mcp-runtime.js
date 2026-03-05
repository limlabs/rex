
globalThis.__rex_mcp_resolved = null;
globalThis.__rex_mcp_rejected = null;

globalThis.__rex_list_mcp_tools = function() {
    var tools = globalThis.__rex_mcp_tools;
    if (!tools) return '[]';
    var result = [];
    var names = Object.keys(tools);
    for (var i = 0; i < names.length; i++) {
        var name = names[i];
        var mod = tools[name];
        result.push({
            name: name,
            description: mod.description || '',
            parameters: mod.parameters || { type: 'object', properties: {} }
        });
    }
    return JSON.stringify(result);
};

globalThis.__rex_call_mcp_tool = function(name, paramsJson) {
    var tools = globalThis.__rex_mcp_tools;
    if (!tools) throw new Error('No MCP tools registered');
    var mod = tools[name];
    if (!mod) throw new Error('MCP tool not found: ' + name);
    var handlerFn = mod.default;
    if (!handlerFn) throw new Error('No default export for MCP tool: ' + name);

    var params = JSON.parse(paramsJson);
    var result = handlerFn(params);
    if (result && typeof result.then === 'function') {
        globalThis.__rex_mcp_resolved = null;
        globalThis.__rex_mcp_rejected = null;
        result.then(
            function(v) { globalThis.__rex_mcp_resolved = v; },
            function(e) { globalThis.__rex_mcp_rejected = e; }
        );
        return '__REX_MCP_ASYNC__';
    }
    return JSON.stringify(result);
};

globalThis.__rex_resolve_mcp = function() {
    if (globalThis.__rex_mcp_rejected) throw globalThis.__rex_mcp_rejected;
    if (globalThis.__rex_mcp_resolved !== null) return JSON.stringify(globalThis.__rex_mcp_resolved);
    throw new Error('MCP tool promise did not resolve');
};
