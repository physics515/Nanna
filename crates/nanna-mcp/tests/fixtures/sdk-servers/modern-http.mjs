// A real @modelcontextprotocol/server 2.0 Streamable HTTP server for
// dual_era_live.rs: `node modern-http.mjs <port> <json|sse> [bearer-token]`.
// Serves the modern (2026-07-28) revision only (`legacy: 'reject'`).
// `POST /grow` adds a tool (`late`) and announces tools/list_changed to every
// open subscriptions/listen stream.
import { createServer } from 'node:http';
import { McpServer, createMcpHandler } from '@modelcontextprotocol/server';
import * as z from 'zod';

const [port, responseMode = 'json', token] = process.argv.slice(2);
let grown = false;
const handler = createMcpHandler(() => {
  const server = new McpServer(
    { name: 'modern-http-fixture', version: '1.0.0' },
    { capabilities: { tools: {} } },
  );
  server.registerTool(
    'shout',
    { description: 'Uppercase text', inputSchema: z.object({ text: z.string() }) },
    async ({ text }) => ({ content: [{ type: 'text', text: text.toUpperCase() }] }),
  );
  server.registerTool(
    'regional',
    {
      description: 'Echo the region header the client mirrored',
      inputSchema: z.object({ region: z.string().meta({ 'x-mcp-header': 'Region' }) }),
    },
    async ({ region }, ctx) => ({ content: [{ type: 'text', text: `region=${region}` }] }),
  );
  if (grown) {
    server.registerTool(
      'late',
      { description: 'Registered after connect', inputSchema: z.object({}) },
      async () => ({ content: [{ type: 'text', text: 'late tool answered' }] }),
    );
  }
  return server;
}, { legacy: 'reject', responseMode });

createServer(async (req, res) => {
  if (token && req.headers.authorization !== `Bearer ${token}`) {
    res.writeHead(401, { 'content-type': 'text/plain' }).end('missing or wrong bearer token');
    return;
  }
  if (req.url === '/grow') {
    grown = true;
    handler.notify.toolsChanged();
    res.writeHead(204).end();
    return;
  }
  const chunks = [];
  for await (const chunk of req) chunks.push(chunk);
  const body = chunks.length ? Buffer.concat(chunks) : undefined;
  const request = new Request(`http://127.0.0.1:${port}${req.url}`, {
    method: req.method, headers: req.headers, body: req.method === 'GET' || req.method === 'HEAD' ? undefined : body,
  });
  const response = await handler.fetch(request);
  res.writeHead(response.status, Object.fromEntries(response.headers));
  if (response.body) {
    for await (const chunk of response.body) res.write(chunk);
  }
  res.end();
}).listen(Number(port), '127.0.0.1', () => console.error(`modern-http on ${port} (${responseMode})`));
