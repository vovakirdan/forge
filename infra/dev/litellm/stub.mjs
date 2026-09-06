import http from 'node:http';

// No network client exists in this fixture: every completion is synthetic.
const server = http.createServer(async (request, response) => {
  if (request.url === '/health') {
    response.writeHead(200).end('forge-contract-stub');
    return;
  }
  if (request.url !== '/v1/chat/completions' || request.method !== 'POST') {
    response.writeHead(404).end();
    return;
  }
  let body = '';
  for await (const chunk of request) {
    body += chunk;
    if (body.length > 1024 * 1024) {
      response.writeHead(413).end();
      return;
    }
  }
  const completion = {
    id: 'chatcmpl-forge-fixture', object: 'chat.completion', created: 1,
    model: 'gpt-4o-mini',
    choices: [{ index: 0, message: { role: 'assistant', content: 'fixture response' }, finish_reason: 'stop' }],
    usage: { prompt_tokens: 1000, completion_tokens: 1000, total_tokens: 2000 },
  };
  let input;
  try { input = JSON.parse(body); } catch {
    response.writeHead(400).end();
    return;
  }
  if (JSON.stringify(input.messages).includes('FORGE_FIXTURE_DELAY')) {
    await new Promise(resolve => setTimeout(resolve, 3000));
  }
  if (input.stream === true) {
    response.writeHead(200, { 'content-type': 'text/event-stream' });
    if (JSON.stringify(input.messages).includes('FORGE_FIXTURE_WAIT')) {
      response.flushHeaders();
      return;
    }
    for (const chunk of [
      { choices: [{ index: 0, delta: { role: 'assistant', content: 'fixture response' }, finish_reason: null }] },
      { choices: [{ index: 0, delta: {}, finish_reason: 'stop' }] },
      { choices: [], usage: completion.usage },
    ]) {
      response.write(`data: ${JSON.stringify({ id: completion.id, object: 'chat.completion.chunk', created: 1, model: completion.model, ...chunk })}\n\n`);
    }
    response.end('data: [DONE]\n\n');
    return;
  }
  response.writeHead(200, { 'content-type': 'application/json' });
  response.end(JSON.stringify(completion));
}).listen(4101, '0.0.0.0');

process.once('SIGTERM', () => {
  server.closeAllConnections();
  server.close(() => process.exit(0));
});
