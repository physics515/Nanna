// A real @modelcontextprotocol/server 2.0 stdio server for dual_era_live.rs.
// `node modern.mjs` is modern-only (it rejects `initialize` with -32022);
// `node modern.mjs dual` also serves the legacy handshake; `node modern.mjs
// grow` registers a second tool (`late`) 1.5 s after the first request, which
// announces a tools/list_changed to any subscriptions/listen stream.
import { McpServer } from '@modelcontextprotocol/server';
import { serveStdio } from '@modelcontextprotocol/server/stdio';
import * as z from 'zod';

const legacy = process.argv[2] === 'dual' ? 'serve' : 'reject';
const grow = process.argv[2] === 'grow';
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
  if (grow) {
    setTimeout(() => {
      server.registerTool(
        'late',
        { description: 'Registered after connect', inputSchema: z.object({}) },
        async () => ({ content: [{ type: 'text', text: 'late tool answered' }] }),
      );
    }, 1500);
  }
  return server;
}, { legacy });
