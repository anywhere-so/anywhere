# Live chat

Deploy with `anywhere deploy samples/live-chat`, open its address, and share it with another person using `anywhere shares add live-chat <email>` or the portal. Both people sign in normally. Messages arrive over WebSockets with the sender's verified Anywhere username. Removing a person's share closes their connection.

The newest 100 messages are saved in `data/messages.json` before being broadcast, and survive redeploys. The page displays messages as text. Refresh after a disconnection to sign in again.

The server uses the MIT-licensed [ws](https://github.com/websockets/ws) package, maintained by Luigi Pinca, pinned with its lockfile. It handles the WebSocket protocol instead of adding a frame parser to this example.
