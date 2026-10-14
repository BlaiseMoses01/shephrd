const assert = require("node:assert");
const { add } = require("./add.js");

assert.strictEqual(add(2, 3), 5);
console.log("all tests passed");
