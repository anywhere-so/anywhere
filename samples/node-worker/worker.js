// A background app on Node.js's standard library alone: it has no web address
// and no port, and logs a line every five seconds for as long as it runs.

const EVERY_MS = 5000;

let ticks = 0;
console.log(`started; logging a line every ${EVERY_MS / 1000} seconds`);

const timer = setInterval(() => {
  ticks += 1;
  console.log(`tick ${ticks} at ${new Date().toISOString()}`);
}, EVERY_MS);

// The platform asks an app to stop with SIGTERM. Once the timer is cleared,
// nothing is left to do, so Node exits.
process.on("SIGTERM", () => {
  clearInterval(timer);
  console.log(`stopping after ${ticks} ticks`);
});
