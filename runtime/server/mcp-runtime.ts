/* eslint-disable @typescript-eslint/no-explicit-any */

declare var __rex_mcp_tools: Record<string, any> | undefined;
declare var __rex_mcp_resolved: any;
declare var __rex_mcp_rejected: any;
declare var __rex_list_mcp_tools: () => string;
declare var __rex_call_mcp_tool: (name: string, paramsJson: string) => string;
declare var __rex_resolve_mcp: () => string;

globalThis.__rex_mcp_resolved = null;
globalThis.__rex_mcp_rejected = null;

globalThis.__rex_list_mcp_tools = function(): string {
    const tools = globalThis.__rex_mcp_tools;
    if (!tools) return '[]';
    const result: { name: string; description: string; parameters: unknown }[] = [];
    const names = Object.keys(tools);
    for (let i = 0; i < names.length; i++) {
        const name = names[i];
        const mod = tools[name];
        result.push({
            name: name,
            description: mod.description || '',
            parameters: mod.parameters || { type: 'object', properties: {} }
        });
    }
    return JSON.stringify(result);
};

globalThis.__rex_call_mcp_tool = function(name: string, paramsJson: string): string {
    const tools = globalThis.__rex_mcp_tools;
    if (!tools) throw new Error('No MCP tools registered');
    const mod = tools[name];
    if (!mod) throw new Error('MCP tool not found: ' + name);
    const handlerFn = mod.default;
    if (!handlerFn) throw new Error('No default export for MCP tool: ' + name);

    const params = JSON.parse(paramsJson);
    const result = handlerFn(params);
    if (result && typeof result.then === 'function') {
        globalThis.__rex_mcp_resolved = null;
        globalThis.__rex_mcp_rejected = null;
        result.then(
            function(v: unknown) { globalThis.__rex_mcp_resolved = v; },
            function(e: unknown) { globalThis.__rex_mcp_rejected = e; }
        );
        return '__REX_MCP_ASYNC__';
    }
    return JSON.stringify(result);
};

globalThis.__rex_resolve_mcp = function(): string {
    if (globalThis.__rex_mcp_rejected) throw globalThis.__rex_mcp_rejected;
    if (globalThis.__rex_mcp_resolved !== null) return JSON.stringify(globalThis.__rex_mcp_resolved);
    throw new Error('MCP tool promise did not resolve');
};
