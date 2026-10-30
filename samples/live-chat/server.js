import { createServer } from "node:http";
import { mkdir, open, readFile, rename } from "node:fs/promises";
import { WebSocket, WebSocketServer } from "ws";

// Anywhere supplies the authenticated username; clients cannot set this header.
await mkdir("data", { recursive: true });
let messages;
try {
  messages = JSON.parse(await readFile("data/messages.json", "utf8"));
  if (!Array.isArray(messages) || messages.length > 100) throw new Error("invalid chat history");
} catch (error) {
  if (error.code !== "ENOENT") throw error;
  messages = [];
}
const page = await readFile(new URL("index.html", import.meta.url));
const server = createServer((request, response) => {
  if (request.method !== "GET" || request.url !== "/") {
    response.writeHead(404).end("Not found");
    return;
  }
  response.writeHead(200, { "content-type": "text/html; charset=utf-8" }).end(page);
});
const sockets = new WebSocketServer({ noServer: true, maxPayload: 4096 });
server.on("upgrade", (request, socket, head) => {
  socket.on("error", (error) => console.warn("client socket failed", error));
  if (request.url !== "/live" || typeof request.headers["anywhere-user"] !== "string") {
    socket.end("HTTP/1.1 403 Forbidden\r\nConnection: close\r\nContent-Length: 0\r\n\r\n");
    return;
  }
  sockets.handleUpgrade(request, socket, head, (client) => {
    sockets.emit("connection", client, request);
  });
});

function send(client, value) {
  if (client.readyState !== WebSocket.OPEN) return;
  if (client.bufferedAmount > 64 * 1024) {
    client.close(1013, "Connection too slow; reopen the app");
    return;
  }
  client.send(JSON.stringify(value), (error) => {
    if (error) {
      console.warn("could not send to a client", error);
      client.terminate();
    }
  });
}

async function save(next) {
  const file = await open("data/messages.next", "w", 0o600);
  try {
    await file.writeFile(JSON.stringify(next));
    await file.sync();
  } finally {
    await file.close();
  }
  await rename("data/messages.next", "data/messages.json");
  const directory = await open("data", "r");
  try {
    await directory.sync();
  } finally {
    await directory.close();
  }
}

let writes = Promise.resolve();
sockets.on("connection", (client, request) => {
  client.on("error", (error) => console.warn("WebSocket client failed", error));
  send(client, { type: "history", messages, username: request.headers["anywhere-user"] });
  client.on("message", (data, binary) => {
    const text = binary ? "" : data.toString().trim();
    if (text.length === 0 || text.length > 1000) {
      client.close(1008, "Send text between 1 and 1000 characters");
      return;
    }
    writes = writes.then(async () => {
      const message = { username: request.headers["anywhere-user"], text, at: new Date().toISOString() };
      const next = [...messages, message].slice(-100);
      await save(next);
      messages = next;
      for (const peer of sockets.clients) send(peer, { type: "message", message });
    }).catch((error) => {
      console.error("could not save chat history", error);
      process.exit(1);
    });
  });
});

// Regular pings keep live connections within the gateway's idle bound.
const heartbeat = setInterval(() => {
  for (const client of sockets.clients) {
    if (client.readyState === WebSocket.OPEN) client.ping();
  }
}, 30_000);
sockets.on("close", () => clearInterval(heartbeat));
server.on("error", (error) => {
  console.error("chat server failed", error);
  process.exit(1);
});
server.listen(Number(process.env.PORT), "0.0.0.0");
