/**
 * Get workflow run outputs (TypeScript)
 * Run: npx tsx docs/api/typescript/get_outputs.ts [workflow_name]
 */
async function getOutputs() {
  const name = process.argv[2] || "api-design-pipeline";
  const resp = await fetch(`http://localhost:3000/api/workflows/outputs?workflowName=${encodeURIComponent(name)}`);
  const data = await resp.json();
  const runs = data.runs || [];
  console.log(`Output runs: ${runs.length}\n`);
  for (const r of runs.slice(0, 5)) {
    console.log(`  ${r.runId?.slice(0, 35)} | ${r.fileCount} files | ${r.totalSize}B`);
    for (const f of (r.files || []).slice(0, 3)) {
      console.log(`    ${f.fileName.padEnd(30)} ${f.sizeBytes}B`);
    }
  }
}

getOutputs().catch(console.error);
