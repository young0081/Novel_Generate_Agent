import assert from "node:assert/strict";
import test from "node:test";

import { isTopDialogLayer, topDialogLayer } from "../src/lib/dialogLayer.ts";

function scopeWith(...layers) {
  return {
    querySelectorAll: () => layers,
  };
}

test("the last mounted modal owns keyboard handling", () => {
  const settings = {};
  const drawer = {};
  const confirm = {};
  const scope = scopeWith(settings, drawer, confirm);

  assert.equal(topDialogLayer(scope), confirm);
  assert.equal(isTopDialogLayer(settings, scope), false);
  assert.equal(isTopDialogLayer(drawer, scope), false);
  assert.equal(isTopDialogLayer(confirm, scope), true);
});

test("an outer dialog resumes ownership after inner layers unmount", () => {
  const settings = {};
  const scope = scopeWith(settings);

  assert.equal(topDialogLayer(scope), settings);
  assert.equal(isTopDialogLayer(settings, scope), true);
  assert.equal(isTopDialogLayer(null, scope), false);
});
