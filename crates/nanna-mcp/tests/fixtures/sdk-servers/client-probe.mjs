// A real @modelcontextprotocol/client 2.0 client, for checking Nanna's MCP
// *server*: `node client-probe.mjs <auto|pin|legacy> <tool> <args-json> -- <command> [args...]`.
// Prints one JSON line: the negotiated version, the tool count, the call result.
import { Client } from '@modelcontextprotocol/client';
import { StdioClientTransport } from '@modelcontextprotocol/client/stdio';

const [mode, tool, argsJson, dashdash, command, ...commandArgs] = process.argv.slice(2);
if (dashdash !== '--') throw new Error('usage: <mode> <tool> <args-json> -- <command> [args...]');
const versionNegotiation =
  mode === 'pin' ? { mode: { pin: '2026-07-28' } } : mode === 'auto' ? { mode: 'auto' } : { mode: 'legacy' };
const client = new Client({ name: 'nanna-probe', version: '1.0.0' }, { versionNegotiation });
const transport = new StdioClientTransport({ command, args: commandArgs, env: process.env, stderr: 'ignore' });
try {
  await client.connect(transport);
  const tools = await client.listTools();
  const result = await client.callTool({ name: tool, arguments: JSON.parse(argsJson) });
  console.log(JSON.stringify({
    version: client.getNegotiatedProtocolVersion(),
    server: client.getServerVersion()?.name,
    tools: tools.tools.length,
    isError: result.isError ?? false,
    text: result.content?.[0]?.text?.slice(0, 120),
  }));
} catch (e) {
  console.log(JSON.stringify({ error: String(e?.message ?? e).slice(0, 300) }));
} finally {
  await client.close().catch(() => {});
}
