Treat the loaded project instructions and AGENTS.md as the SunSwitcher repository contract.

## SunMCP results in codemode

When calling SunMCP tools from codemode:

- Call tools with the mcp__sunmcp__ prefix.
- Treat SunMCP text output as the canonical tool result. Render content text blocks directly and in order. Do not pass the whole CallToolResult to text() or otherwise JSON-serialize it unless the transport/result structure itself is being inspected.
- Preserve SunMCP status and diagnostics, including Status, warnings, truncation metadata, Did_you_mean, and partial/error information.
- Prefer SunMCP-native filtering and selection parameters over filtering, truncating, or reshaping results in codemode.
- Do not use codemode as a second rendering/filtering layer when the SunMCP contract can express the request directly.
- Keep get_tool_info focused: request only needed tools; use full only when exact field-level details or examples are required; tool names passed to SunMCP discovery are unprefixed.

## Agent skills in Pi

- Use Pi's native skill discovery and built-in read tool to read the discovered SKILLs.
- Do not use SunMCP read_skill to load Agent Skills in Pi. Pi externally truncates that MCP result, so it is not the authoritative skill-loading path.
- Treat the workspace-pinned skills under .agents/skills/ as the authoritative project skill copies for the resolved sun-observations version.

Example:

~~~js
const result = await tools.mcp__sunmcp__read_memory_index({});
for (const block of result.content ?? []) {
if (block.type === "text") text(block.text);
}
~~~
