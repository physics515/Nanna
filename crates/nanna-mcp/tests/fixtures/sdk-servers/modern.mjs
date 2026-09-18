// A real @modelcontextprotocol/server 2.0 stdio server for dual_era_live.rs.
// `node modern.mjs` is modern-only (it rejects `initialize` with -32022);
// `node modern.mjs dual` also serves the legacy handshake.
import { McpServer } from '@modelcontextprotocol/server';
import { serveStdio } from '@modelcontextprotocol/server/stdio';
import * as z from 'zod';

const legacy = process.argv[2] === 'dual' ? 'serve' : 'reject';
serveStdio(() => {
  const server = new McpServer(
    { name: 'modern-fixture', version: '1.0.0' },
    { capabilities: { tools: {} } },
  );
  server.registerTool(
    'shout',
    { description: 'Uppercase text', inputSchema: z.object({ text: z.string() }) },
    async ({ text }) => ({ content: [{ type: 'text', text: text.toUpperCase() }] }),
  );
  return server;
}, { legacy });
