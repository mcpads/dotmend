import { test as base, expect } from "@playwright/test";
import { spawn } from "node:child_process";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { createInterface } from "node:readline";
import { randomUUID } from "node:crypto";
import { createConnection } from "node:net";

const binary = resolve(process.env.DOTMEND_TEST_BINARY || `target/debug/dotmend${process.platform === "win32" ? ".exe" : ""}`);
function mcpClient(workspace, runtimeDirectory=join(workspace,"runtime"), perRequestMetadata=true) {
  const child = spawn(binary, ["--workspace", workspace], { stdio: ["pipe", "pipe", "pipe"], env:{...process.env,DOTMEND_RUNTIME_DIR:runtimeDirectory} });
  const controlId = randomUUID();
  const pending = new Map(); let sequence = 0; let stderr = "";
  child.stderr.on("data", data => stderr += data);
  createInterface({ input: child.stdout }).on("line", line => {
    const message = JSON.parse(line); expect(message.jsonrpc).toBe("2.0"); expect("result" in message).not.toBe("error" in message); if(message.result)expect(message.result.resultType).toBe("complete"); const call = pending.get(message.id);
    if (call) { pending.delete(message.id); clearTimeout(call.timer); message.error ? call.reject(new Error(JSON.stringify(message.error))) : call.resolve(message.result); }
  });
  child.on("exit", code => { for (const call of pending.values()) { clearTimeout(call.timer); call.reject(new Error(`MCP exited ${code}: ${stderr}`)); } pending.clear(); });
  const request = (method, params, attachMetadata=true) => new Promise((resolve, reject) => {
    if (child.exitCode !== null) { reject(new Error(`MCP exited before ${method}: ${child.exitCode}: ${stderr}`)); return; }
    if(perRequestMetadata && attachMetadata) params={...params,_meta:{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientInfo":{name:"dotmend-wire-check",version:"test"},"io.modelcontextprotocol/clientCapabilities":{}}};
    const id = ++sequence; const timer = setTimeout(() => { pending.delete(id); reject(new Error(`MCP timeout: ${method}: ${stderr}`)); }, 15000);
    pending.set(id, { resolve, reject, timer }); child.stdin.write(JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n");
  });
  return { child, request, controlId, async discover() { return request("server/discover", {}); }, async tool(name, arguments_) { if(["open_workbench","inspect_workbench","close_workbench","present_art"].includes(name))arguments_={control_id:controlId,...arguments_}; return request("tools/call", { name, arguments: arguments_ }); } };
}
const test = base.extend({
  workbench: async ({ page }, use) => {
    const workspace = await mkdtemp(join(tmpdir(), "dotmend-test-"));
    const client = mcpClient(workspace);
    try {
      await client.discover();
      const opened=(await client.tool("open_workbench",{})).structuredContent;
      expect(opened.ok).toBe(true);
      client.workbenchId=opened.instance.workbench_id;
      const url=opened.instance.url;
      await page.goto(url);
      const errors = []; page.on("pageerror", error => errors.push(error.message));
      await use({ url, client, workspace, errors });
      expect(errors).toEqual([]);
    } finally {
      for (const child of [client.child]) { child.kill(); await new Promise(resolve => child.exitCode !== null || child.signalCode !== null ? resolve() : child.once("exit", resolve)); }
      await rm(workspace, { recursive: true, force: true });
    }
  },
});
async function create(client,name){return (await client.tool("create_art",{target:{resource_id:name,width:8,height:8,palette:["#000000","#000000","#E07440"],transparent_index:0,allowed_indices:[0,1,2],constraints_ref:null,requirements:[]},initial:{kind:"fill",index:0}})).structuredContent.art_id;}
async function inspect(client){return (await client.tool("inspect_presentation",{})).structuredContent.presentation;}
async function show(client,ids,extra={}){const previous=await inspect(client);const args={title:"Refine the hair",note:"Keep the face unchanged and gently refine the hair.",items:ids.map((art_id,i)=>({kind:"art",art_id,label:i?"Reference image":"Image to edit",region:{x:0,y:0,width:8,height:8},scale:32,editable:i===0})),expected_presentation_id:previous?.presentation_id??null,expected_state_id:previous?.state_id??null,...extra};const result=(await client.tool("present_art",{workbench_id:client.workbenchId,view:args})).structuredContent;expect(result.ok).toBe(true);return result.presentation;}
async function settle(page,title="Refine the hair"){await expect(page.locator("#title")).toHaveText(title);await expect(page.locator(".editable")).toBeVisible();}
async function color(page){await page.locator('.palette [data-index="2"]').click();}
async function point(page,x,y){await page.locator(".editable").click({position:{x:x*32+16,y:y*32+16}});}
async function rows(client,id){return (await client.tool("inspect_art",{art_id:id,include_indices:true})).structuredContent.indices;}
async function result(client){return (await inspect(client)).state.art_ids[0];}
const modifier=process.platform==="darwin"?"Meta":"Control";

test("palette painting, one undo and save remain available alongside issue marking",async({page,workbench},testInfo)=>{
  await expect(page).toHaveTitle("Dotmend");
  const id=await create(workbench.client,"face");await show(workbench.client,[id]);await settle(page);
  await expect(page.getByRole("textbox")).toHaveCount(0);await expect(page.getByRole("combobox")).toHaveCount(0);await expect(page.locator('button:not(.swatch)')).toHaveText(["Mark issues",/Undo/,"Save"]);
  await point(page,1,1);await expect(page.locator("#status")).toContainText("Choose a color below");expect(await result(workbench.client)).toBe(id);
  await color(page);await point(page,1,1);await expect.poll(()=>result(workbench.client)).not.toBe(id);const first=await result(workbench.client);expect((await rows(workbench.client,first))[1][1]).toBe(2);
  await expect(page.locator("#undo")).toBeEnabled();await point(page,2,1);await expect.poll(()=>result(workbench.client)).not.toBe(first);
  await expect(page.locator("#undo")).toBeEnabled();await page.keyboard.press(`${modifier}+z`);await expect.poll(()=>result(workbench.client)).toBe(first);await expect(page.locator("#undo")).toBeDisabled();await page.keyboard.press(`${modifier}+z`);expect(await result(workbench.client)).toBe(first);
  await expect(page.locator("#save")).toBeEnabled();await page.keyboard.press(`${modifier}+s`);await expect(page.locator("#status")).toHaveText("Saved.");expect((await inspect(workbench.client)).saved.art_ids[0]).toBe(first);
  await page.reload();await settle(page);await expect(page.locator("#save")).toBeDisabled();expect(await result(workbench.client)).toBe(first);await page.screenshot({path:testInfo.outputPath("simple-editor.png"),fullPage:true});
});
test("only the agent changes the collection, filter and old candidate being shown",async({page,workbench})=>{
  const a=await create(workbench.client,"left"),b=await create(workbench.client,"right");await show(workbench.client,[a,b]);await settle(page);await expect(page.locator(".picture")).toHaveCount(2);await expect(page.locator(".palette")).toHaveCount(1);
  await color(page);await point(page,1,1);await expect.poll(()=>result(workbench.client)).not.toBe(a);const edited=await result(workbench.client);
  const previous=await inspect(workbench.client);await show(workbench.client,[b],{title:"Only the image on the right"});await settle(page,"Only the image on the right");await expect(page.locator(".picture")).toHaveCount(1);expect(await result(workbench.client)).toBe(b);
  await show(workbench.client,[a],{title:"Before editing"});await settle(page,"Before editing");expect((await rows(workbench.client,a))[1][1]).toBe(0);expect((await rows(workbench.client,edited))[1][1]).toBe(2);
  const stored=(await workbench.client.tool("inspect_presentation",{presentation_id:previous.presentation_id})).structuredContent.presentation;expect(stored.state.art_ids[0]).toBe(edited);expect(stored.is_current).toBe(false);
});
test("a protected click leaves the entire stroke unchanged and a valid click still works",async({page,workbench})=>{
  const id=await create(workbench.client,"guard");const selection=(await workbench.client.tool("create_selection",{art_id:id,region:{x:1,y:1,width:1,height:1},selector:{kind:"rect"}})).structuredContent;
  const request=(await workbench.client.tool("request_edit",{base_art_id:id,write_region:{x:0,y:0,width:4,height:4},instruction:"Preserve the face",protected_selection_ids:[selection.selection_id]})).structuredContent;
  await show(workbench.client,[id],{items:[{kind:"art",art_id:id,label:"Image to edit",region:{x:0,y:0,width:8,height:8},scale:32,editable:true,request_id:request.request_id}]});await settle(page);await color(page);await point(page,1,1);
  await expect(page.locator("#status")).toContainText("This area is protected");expect(await result(workbench.client)).toBe(id);const rgba=await page.locator(".editable").evaluate(c=>Array.from(c.getContext("2d").getImageData(48,48,1,1).data));expect(rgba).toEqual([237,240,233,255]);
  await point(page,2,2);await expect.poll(()=>result(workbench.client)).not.toBe(id);const modified=await result(workbench.client);expect((await rows(workbench.client,modified))[1][1]).toBe(0);
  await expect(page.locator("#undo")).toBeEnabled();await page.keyboard.press(`${modifier}+z`);await expect.poll(()=>result(workbench.client)).toBe(id);await expect(page.locator(".palette .swatch").first()).toBeEnabled();await point(page,1,1);await expect(page.locator("#status")).toContainText("This area is protected");expect(await result(workbench.client)).toBe(id);
});
test("cropped editable and reference views keep different source coordinates",async({page,workbench})=>{
  const id=await create(workbench.client,"cropped"),render=await workbench.client.tool("render_art",{art_id:id,scale:4});const png=Buffer.from(render.content.find(c=>c.type==="image").data,"base64");
  const source={source_path:"reference.png"};await writeFile(join(workbench.workspace,source.source_path),png);const attachedResult=(await workbench.client.tool("attach_reference",{art_id:id,source_path:source.source_path,label:"Reference",role:"reference"})).structuredContent;const attached=attachedResult.art_id;
  const attachedArt=(await workbench.client.tool("inspect_art",{art_id:attached})).structuredContent;source.source_hash=attachedArt.references[0].source_hash;
  await show(workbench.client,[attached],{items:[{kind:"art",art_id:attached,label:"Small region",region:{x:4,y:3,width:3,height:3},scale:32,editable:true},{kind:"reference",art_id:attached,source_hash:source.source_hash,label:"Original reference",region:{x:16,y:12,width:12,height:12},scale:4}]});await settle(page);await expect(page.locator(".picture")).toHaveCount(2);
  await color(page);await point(page,1,1);await expect.poll(()=>result(workbench.client)).not.toBe(attached);expect((await rows(workbench.client,await result(workbench.client)))[4][5]).toBe(2);
  const before=await result(workbench.client);await page.locator('.picture[data-item="1"] canvas').click();expect(await result(workbench.client)).toBe(before);
});
test("a stale browser action cannot write onto an agent's new presentation",async({page,workbench})=>{
  const a=await create(workbench.client,"a"),b=await create(workbench.client,"b");await show(workbench.client,[a]);await settle(page);await color(page);
  let release;const blocked=new Promise(resolve=>release=resolve);let reached;const entered=new Promise(resolve=>reached=resolve);
  await page.route("**/api/presentation/action",async route=>{reached();await blocked;await route.continue();});
  await point(page,1,1);await entered;await show(workbench.client,[b],{title:"Another image"});release();await settle(page,"Another image");await expect(page.locator("#status")).toContainText("New artwork has arrived");expect((await rows(workbench.client,a))[1][1]).toBe(0);expect((await rows(workbench.client,b))[1][1]).toBe(0);
});
test("a lost save response is recovered without inventing a new result",async({page,workbench})=>{
  const a=await create(workbench.client,"save");await show(workbench.client,[a]);await settle(page);await color(page);await point(page,1,1);await expect.poll(()=>result(workbench.client)).not.toBe(a);const painted=await result(workbench.client);
  await page.route("**/api/presentation/action",async route=>{await route.fetch();await route.abort();},{times:1});await page.locator("#save").click();await expect.poll(async()=>(await inspect(workbench.client)).saved?.art_ids[0]).toBe(painted);
  await expect(page.locator("#status")).toHaveText("Saved.");
  await page.reload();await settle(page);await expect(page.locator("#status")).toHaveText("Saved.");await expect(page.locator("#save")).toBeDisabled();expect(await result(workbench.client)).toBe(painted);
});
test("the agent records a user's explicit feedback while save alone does not approve art",async({page,workbench})=>{
  const id=await create(workbench.client,"review");const request=(await workbench.client.tool("request_edit",{base_art_id:id,write_region:{x:0,y:0,width:8,height:8},instruction:"For human review"})).structuredContent;
  await workbench.client.tool("submit_edit_result",{request_id:request.request_id,result_art_id:id,notes:"Propose the current candidate"});await show(workbench.client,[id]);await settle(page);await page.locator("#save").click();await expect(page.locator("#save")).toBeDisabled();
  expect((await workbench.client.tool("inspect_edit_request",{request_id:request.request_id})).structuredContent.human_review).toBe("pending");
  const reviewed=await workbench.client.tool("review_edit_result",{request_id:request.request_id,result_art_id:id,decision:"changes_requested",notes:"Synthetic user feedback: reduce the bright spots in the hair",regions:[{region:{x:1,y:1,width:2,height:2},comment:"Reduce only this spot"}],expected_review_id:null});expect(reviewed.structuredContent.human_review).toBe("changes_requested");await expect(page.getByRole("textbox")).toHaveCount(0);
});

test("the agent starts and stops a timed frame view without adding human controls",async({page,workbench})=>{
  const a=await create(workbench.client,"walk");const b=(await workbench.client.tool("edit_art",{art_id:a,write_region:{x:0,y:0,width:8,height:8},operations:[{kind:"set_pixels",pixels:[{x:0,y:0,index:2}]}]})).structuredContent.art_id;
  await show(workbench.client,[a],{title:"Walking comparison",items:[{kind:"art",art_id:a,label:"Step",region:{x:0,y:0,width:8,height:8},scale:32,editable:false,playback:[{art_id:a,label:"Idle",duration_ms:200},{art_id:b,label:"Walking",duration_ms:200}]}]});await expect(page.locator("#title")).toHaveText("Walking comparison");await expect(page.locator('button:not(.swatch)')).toHaveText(["Mark issues",/Undo/,"Save"]);await expect(page.locator("#save")).toBeDisabled();await expect(page.locator(".palette")).toHaveCount(0);
  const pixel=()=>page.locator("canvas").evaluate(c=>Array.from(c.getContext("2d").getImageData(16,16,1,1).data));await expect.poll(pixel,{intervals:[40]}).toEqual([224,116,64,255]);await expect.poll(pixel,{intervals:[40]}).not.toEqual([224,116,64,255]);
  await show(workbench.client,[a],{title:"Still image"});await settle(page,"Still image");
  const observed=await page.locator("canvas").evaluate(async canvas=>{
    const samples=[];const deadline=performance.now()+600;
    while(performance.now()<deadline){samples.push(Array.from(canvas.getContext("2d").getImageData(16,16,1,1).data));await new Promise(resolve=>requestAnimationFrame(resolve));}
    return samples;
  });
  expect(observed.length).toBeGreaterThan(0);for(const rgba of observed)expect(rgba).toEqual([237,240,233,255]);
});

for(const failure of [false,true])test(`a delayed ${failure?"failed":"successful"} poll cannot replace a saved edit`,async({page,workbench})=>{
  const id=await create(workbench.client,"delayed-poll");await show(workbench.client,[id]);await settle(page);await color(page);
  let release;const blocked=new Promise(resolve=>release=resolve);let reached;const entered=new Promise(resolve=>reached=resolve);
  await page.route("**/api/presentation",async route=>{const old=await route.fetch();reached();await blocked;if(failure)await route.abort();else await route.fulfill({response:old});},{times:1});
  await entered;await point(page,1,1);await expect.poll(()=>result(workbench.client)).not.toBe(id);const edited=await result(workbench.client);
  await page.locator("#save").click();await expect(page.locator("#status")).toHaveText("Saved.");
  const delivered=page.waitForEvent(failure?"requestfailed":"requestfinished",request=>request.url()===`${workbench.url}/api/presentation`);release();await delivered;
  await page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));
  await expect(page.locator("#status")).toHaveText("Saved.");await expect(page.locator("#save")).toBeDisabled();
  expect(await page.locator("canvas").evaluate(c=>Array.from(c.getContext("2d").getImageData(48,48,1,1).data))).toEqual([224,116,64,255]);expect(await result(workbench.client)).toBe(edited);
});

async function stopClient(client,signal){
  if(client.child.exitCode!==null||client.child.signalCode!==null)return;
  const exited=new Promise(resolve=>client.child.once("exit",resolve));
  if(signal)client.child.kill(signal);else client.child.stdin.end();
  await exited;
}
async function instance(client,args={}){const result=(await client.tool("open_workbench",args)).structuredContent;expect(result.ok).toBe(true);client.workbenchId=result.instance.workbench_id;return result.instance;}
async function isServing(page,url){try{return (await page.request.get(url+"/api/workbench",{timeout:500})).ok();}catch{return false;}}

test("repeated MCP open requests reuse one owned server",async({workbench})=>{
  const opened=await Promise.all(Array.from({length:20},()=>workbench.client.tool("open_workbench",{})));
  for(const response of opened){expect(response.structuredContent.ok).toBe(true);expect(response.structuredContent.instance.workbench_id).toBe(workbench.client.workbenchId);expect(response.structuredContent.instance.url).toBe(workbench.url);}
});

test("another MCP owner can create candidates but cannot present or close the shared screen",async({page,workbench})=>{
  const other=mcpClient(workbench.workspace);await other.discover();
  try{
    const id=await create(workbench.client,"ownership");await show(workbench.client,[id]);await settle(page);
    expect((await other.tool("open_workbench",{})).structuredContent.error.code).toBe("workbench_busy");
    expect((await other.tool("inspect_workbench",{})).structuredContent.state).toBe("busy");
    await create(other,"parallel-candidate");
    const old=await inspect(workbench.client);
    expect((await other.tool("present_art",{workbench_id:workbench.client.workbenchId,view:old.presentation})).structuredContent.error.code).toBe("workbench_not_owned");
    expect((await other.tool("close_workbench",{workbench_id:workbench.client.workbenchId})).structuredContent.error.code).toBe("workbench_not_owned");
    const oldId=workbench.client.workbenchId;
    await workbench.client.tool("close_workbench",{workbench_id:oldId});
    await expect.poll(()=>isServing(page,workbench.url)).toBe(false);
    const next=await instance(other);expect(next.workbench_id).not.toBe(oldId);
    await workbench.client.tool("close_workbench",{workbench_id:oldId});
    expect(await isServing(page,next.url)).toBe(true);
    expect((await workbench.client.tool("present_art",{workbench_id:oldId,view:old.presentation})).structuredContent.error.code).toBe("workbench_not_owned");
    const stale=await page.request.post(next.url+"/api/presentation/action",{headers:{"x-retro-art-workbench":oldId},data:{action:"save",presentation_id:old.presentation_id,expected_state_id:old.state_id}});
    expect((await stale.json()).error.code).toBe("workbench_conflict");expect((await inspect(other)).saved).toBeNull();
  }finally{await stopClient(other);}
});

test("simultaneous MCP starts give one workspace to exactly one owner",async({workbench})=>{
  await workbench.client.tool("close_workbench",{workbench_id:workbench.client.workbenchId});
  const clients=[mcpClient(workbench.workspace),mcpClient(workbench.workspace)];
  try{
    await Promise.all(clients.map(c=>c.discover()));
    const results=await Promise.all(clients.map(c=>c.tool("open_workbench",{})));
    expect(results.filter(r=>r.structuredContent.ok)).toHaveLength(1);
    expect(results.filter(r=>r.structuredContent.error?.code==="workbench_busy")).toHaveLength(1);
  }finally{await Promise.all(clients.map(c=>stopClient(c)));}
});

test("the shared instance cap prevents proliferation across workspaces and releases capacity",async({workbench})=>{
  const clients=Array.from({length:4},(_,i)=>mcpClient(join(workbench.workspace,`workspace-${i}`),join(workbench.workspace,"runtime")));
  try{
    await Promise.all(clients.map(c=>c.discover()));
    for(const client of clients.slice(0,3))await instance(client);
    expect((await clients[3].tool("open_workbench",{})).structuredContent.error.code).toBe("workbench_limit");
    await clients[0].tool("close_workbench",{workbench_id:clients[0].workbenchId});
    await instance(clients[3]);
    expect((await workbench.client.tool("inspect_workbench",{})).structuredContent.state).toBe("owned");
  }finally{await Promise.all(clients.map(c=>stopClient(c)));}
});

test("MCP disconnect and process death close HTTP and preserve saved edits for a new owner",async({page,workbench})=>{
  const id=await create(workbench.client,"reconnect");await show(workbench.client,[id]);await settle(page);await color(page);await point(page,1,1);await expect.poll(()=>result(workbench.client)).not.toBe(id);
  await page.locator("#save").click();await expect(page.locator("#save")).toBeDisabled();const saved=(await inspect(workbench.client)).saved;
  await stopClient(workbench.client);await expect.poll(()=>isServing(page,workbench.url)).toBe(false);
  let previous=workbench.client.workbenchId;
  for(const signal of ["SIGKILL",undefined]){
    const client=mcpClient(workbench.workspace);await client.discover();
    try{const opened=await instance(client);expect(opened.workbench_id).not.toBe(previous);previous=opened.workbench_id;expect((await inspect(client)).saved).toEqual(saved);await stopClient(client,signal);await expect.poll(()=>isServing(page,opened.url)).toBe(false);}
    finally{await stopClient(client);}
  }
});

test("browser polling does not prevent idle shutdown and reopening keeps the draft",async({page,workbench})=>{
  const id=await create(workbench.client,"idle");await show(workbench.client,[id]);const draft=await inspect(workbench.client);
  await workbench.client.tool("close_workbench",{workbench_id:workbench.client.workbenchId});
  const short=await instance(workbench.client,{idle_timeout_seconds:2});await page.goto(short.url);await settle(page);
  await page.evaluate(()=>setInterval(()=>document.dispatchEvent(new PointerEvent("pointermove",{bubbles:true})),100));
  await expect.poll(async()=>(await workbench.client.tool("inspect_workbench",{})).structuredContent.state,{timeout:7000}).toBe("closed");
  expect(await isServing(page,short.url)).toBe(false);
  const renewed=await instance(workbench.client);expect(renewed.workbench_id).not.toBe(short.workbench_id);expect((await inspect(workbench.client)).state_id).toBe(draft.state_id);
  expect((await workbench.client.tool("close_workbench",{workbench_id:short.workbench_id})).structuredContent.error.code).toBe("workbench_conflict");
  expect(await isServing(page,renewed.url)).toBe(true);
  expect((await (await page.request.get(renewed.url+"/api/workbench")).json()).workbench_id).toBe(renewed.workbench_id);
});

for(const activity of ["scrolling","choosing a color","dragging"])test(`${activity} renews idle time without changing the workbench address`,async({page,workbench})=>{
  await page.setViewportSize({width:600,height:1000});
  const id=await create(workbench.client,"active-view");
  await show(workbench.client,[id],{items:[{kind:"art",art_id:id,label:"Image to edit",region:{x:0,y:0,width:8,height:8},scale:64,editable:true}]});
  const original=await inspect(workbench.client);
  await workbench.client.tool("close_workbench",{workbench_id:workbench.client.workbenchId});
  const opened=await instance(workbench.client,{idle_timeout_seconds:2});await page.goto(opened.url);await settle(page);
  const canvas=page.locator("canvas.markable"),box=await canvas.boundingBox();
  if(activity==="scrolling")await page.locator(".drawing").hover();
  if(activity==="dragging"){
    await page.locator("#mark").click();await page.mouse.move(box.x+32,box.y+32);await page.mouse.down();
  }
  // Keep interacting past the original deadline without completing a stored action.
  for(let i=0;i<8;i++){
    await page.waitForTimeout(400);
    if(activity==="scrolling")await page.mouse.wheel(i%2?-24:24,0);
    else if(activity==="choosing a color")await color(page);
    else await page.mouse.move(box.x+32+(i%2)*64,box.y+32);
  }
  const current=(await workbench.client.tool("inspect_workbench",{})).structuredContent;
  expect(current.state).toBe("owned");expect(current.instance).toEqual(opened);
  expect((await inspect(workbench.client)).state_id).toBe(original.state_id);
  if(activity==="dragging"){
    await page.mouse.up();await expect.poll(async()=>(await marks(workbench.client))[0]?.pixels).toEqual([{x:0,y:0},{x:1,y:0}]);
  }
  // Stopping real input must still release an unattended screen despite browser polling.
  await expect.poll(async()=>(await workbench.client.tool("inspect_workbench",{})).structuredContent.state,{timeout:7000}).toBe("closed");
});

test("HTTP cannot bypass managed MCP ownership for tool mutations",async({page,workbench})=>{
  const id=await create(workbench.client,"http");await show(workbench.client,[id]);const view=await inspect(workbench.client);
  const response=await page.request.post(workbench.url+"/api/call",{data:{tool:"present_art",arguments:{workbench_id:workbench.client.workbenchId,view:view.presentation}}});
  expect((await response.json()).error.code).toBe("mcp_required");expect((await inspect(workbench.client)).state_id).toBe(view.state_id);
  const noIdentity=await page.request.post(workbench.url+"/api/presentation/action",{data:{action:"save",presentation_id:view.presentation_id,expected_state_id:view.state_id}});
  expect((await noIdentity.json()).error.code).toBe("workbench_conflict");
});

test("an owning agent renews the same address across connections while continuing work",async({page,workbench})=>{
  await workbench.client.tool("close_workbench",{workbench_id:workbench.client.workbenchId});
  const opened=await instance(workbench.client,{idle_timeout_seconds:2});
  const other=mcpClient(workbench.workspace);await other.discover();
  try{
    for(let i=0;i<4;i++){
      await page.waitForTimeout(650);
      const renewed=(await other.tool("open_workbench",{control_id:workbench.client.controlId,idle_timeout_seconds:1})).structuredContent;
      expect(renewed.state).toBe("owned");expect(renewed.instance).toEqual(opened);
    }
    await stopClient(other);
    expect(await isServing(page,opened.url)).toBe(true);
    await expect.poll(async()=>(await workbench.client.tool("inspect_workbench",{})).structuredContent.state,{timeout:7000}).toBe("closed");
  }finally{await stopClient(other);}
});

test("unrelated art calls cannot keep an unattended workbench alive",async({page,workbench})=>{
  const id=await create(workbench.client,"agent-activity");
  await workbench.client.tool("close_workbench",{workbench_id:workbench.client.workbenchId});
  const opened=await instance(workbench.client,{idle_timeout_seconds:2});
  const other=mcpClient(workbench.workspace);await other.discover();
  try{
    for(let i=0;i<6;i++){
      await page.waitForTimeout(500);
      const result=await other.tool(i%2?"render_art":"inspect_art",{art_id:id});expect(result.structuredContent.ok).toBe(true);
    }
    const current=(await workbench.client.tool("inspect_workbench",{})).structuredContent;
    expect(current.state).toBe("closed");
    expect(await isServing(page,opened.url)).toBe(false);
  }finally{await stopClient(other);}
});

test("declared agent work survives external waiting and returns to idle at handoff",async({page,workbench})=>{
  await workbench.client.tool("close_workbench",{workbench_id:workbench.client.workbenchId});
  const opened=await instance(workbench.client,{idle_timeout_seconds:1,work_state:"working"});
  const other=mcpClient(workbench.workspace);await other.discover();
  try{
    await page.waitForTimeout(2200);
    const current=(await other.tool("inspect_workbench",{control_id:workbench.client.controlId})).structuredContent;
    expect(current.state).toBe("owned");expect(current.instance).toEqual(opened);expect(current.work_state).toBe("working");
    expect((await other.tool("open_workbench",{control_id:workbench.client.controlId})).structuredContent.work_state).toBe("working");
    expect((await other.tool("open_workbench",{work_state:"waiting"})).structuredContent.error.code).toBe("workbench_busy");
    const handoff=(await other.tool("open_workbench",{control_id:workbench.client.controlId,work_state:"waiting"})).structuredContent;
    expect(handoff.instance).toEqual(opened);expect(handoff.work_state).toBe("waiting");
    for(let i=0;i<7;i++){
      await page.waitForTimeout(250);
      await other.tool("inspect_presentation",{});
      expect((await other.tool("inspect_art",{art_id:"missing"})).isError).toBe(true);
    }
    expect((await workbench.client.tool("inspect_workbench",{})).structuredContent.state).toBe("closed");
  }finally{await stopClient(other);}
});

test("a stalled committed save recovers without reloading or replaying it",async({page,workbench})=>{
  const id=await create(workbench.client,"stalled-save");await show(workbench.client,[id]);await settle(page);await color(page);await point(page,1,1);
  await expect.poll(()=>result(workbench.client)).not.toBe(id);
  let requests=0;
  page.on("request",request=>{if(request.url().endsWith("/api/presentation/action"))requests++;});
  await page.evaluate(()=>{
    const fetch=window.fetch;
    let release;const blocked=new Promise(resolve=>release=resolve);
    window.delayedSave={ready:false,settled:false,release};
    window.fetch=async(...args)=>{
      const response=await fetch(...args);
      if(String(args[0]).endsWith("/api/presentation/action")&&JSON.parse(args[1].body).action==="save"){
        window.fetch=fetch;
        const data=await response.json();
        // Delay parsed data so abort cannot prevent its eventual completion.
        response.json=async()=>{window.delayedSave.ready=true;await blocked;window.delayedSave.settled=true;return data;};
      }
      return response;
    };
  });
  try{
    await page.locator("#save").click();await expect.poll(()=>page.evaluate(()=>window.delayedSave.ready)).toBe(true);
    const saved=(await inspect(workbench.client)).saved;
    await expect(page.locator("#status")).toHaveText("Saved.",{timeout:12000});
    await expect(page.locator("#mark")).toBeEnabled();expect(requests).toBe(1);
    await point(page,2,1);await expect.poll(()=>result(workbench.client)).not.toBe(saved.art_ids[0]);
    await expect(page.locator("#status")).toHaveText("Click Save when you are ready.");
    const edited=await result(workbench.client);
    await page.evaluate(async()=>{window.delayedSave.release();await new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));});
    expect(await page.evaluate(()=>window.delayedSave.settled)).toBe(true);
    await expect(page.locator("#status")).toHaveText("Click Save when you are ready.");await expect(page.locator("#save")).toBeEnabled();
    expect(await page.locator("canvas").evaluate(c=>Array.from(c.getContext("2d").getImageData(80,48,1,1).data))).toEqual([224,116,64,255]);
    expect(await result(workbench.client)).toBe(edited);expect((await inspect(workbench.client)).saved).toEqual(saved);expect(requests).toBe(2);
  }finally{await page.evaluate(()=>window.delayedSave.release());}
});

for(const lostResponse of [false,true])test(`a ${lostResponse?"recovered":"confirmed"} edit follows new artwork after its image fails to load`,async({page,workbench})=>{
  const id=await create(workbench.client,"failed-edit-image"),replacement=await create(workbench.client,"replacement-image");
  await show(workbench.client,[id]);await settle(page);await color(page);
  let failedArt;
  await page.route("**/api/art/*",route=>{
    const artId=route.request().url().split("/").pop();
    if(artId!==id&&artId!==replacement){failedArt=artId;return route.fulfill({status:503,json:{ok:false,error:{code:"source_unavailable",message:"Temporary asset failure"}}});}
    return route.continue();
  });
  if(lostResponse)await page.route("**/api/presentation/action",async route=>{await route.fetch();await route.abort();},{times:1});
  await point(page,1,1);await expect.poll(()=>result(workbench.client)).not.toBe(id);const edited=await result(workbench.client);
  await expect(page.locator("#status")).toContainText("Connection lost");await expect(page.locator("#mark")).toBeDisabled();expect(failedArt).toBe(edited);
  await show(workbench.client,[replacement],{title:"Replacement available"});
  await settle(page,"Replacement available");await expect(page.locator("#mark")).toBeEnabled();
  expect((await rows(workbench.client,edited))[1][1]).toBe(2);
  await page.unroute("**/api/art/*");await color(page);await point(page,2,1);
  await expect(page.locator("#undo")).toBeEnabled();
  expect((await rows(workbench.client,await result(workbench.client)))[1]).toEqual([0,0,2,0,0,0,0,0]);
});

test("recovery fences an undelivered stroke before allowing another edit",async({page,workbench})=>{
  const id=await create(workbench.client,"late-stroke");await show(workbench.client,[id]);await settle(page);await color(page);
  let release;const blocked=new Promise(resolve=>release=resolve);let reached;const entered=new Promise(resolve=>reached=resolve);let request;
  await page.route("**/api/presentation/action",async route=>{request=route.request();reached();await blocked;await route.abort().catch(()=>{});},{times:1});
  try{
    await point(page,1,1);await entered;
    await expect(page.locator("#status")).toContainText("was not applied",{timeout:12000});
    await expect(page.locator("#mark")).toBeEnabled();expect(await result(workbench.client)).toBe(id);
    const late=await page.request.post(request.url(),{headers:request.headers(),data:request.postDataJSON()});
    expect((await late.json()).error.code).toBe("action_cancelled");expect(await result(workbench.client)).toBe(id);
    await point(page,2,1);await expect.poll(()=>result(workbench.client)).not.toBe(id);
    expect((await rows(workbench.client,await result(workbench.client)))[1]).toEqual([0,0,2,0,0,0,0,0]);
  }finally{release();}
});

test("a disconnected screen disables editing and recovers after reconnection",async({page,workbench})=>{
  const id=await create(workbench.client,"connection");await show(workbench.client,[id]);await settle(page);await color(page);
  await page.context().setOffline(true);
  await expect(page.locator("#status")).toContainText("Connection lost");
  await expect(page.locator("#mark")).toBeDisabled();await expect(page.locator(".swatch").first()).toBeDisabled();
  await point(page,1,1);expect(await result(workbench.client)).toBe(id);
  await page.context().setOffline(false);await expect(page.locator("#mark")).toBeEnabled();
  await point(page,1,1);await expect.poll(()=>result(workbench.client)).not.toBe(id);
  await workbench.client.tool("close_workbench",{workbench_id:workbench.client.workbenchId});
  // Shutdown can reject an in-flight request or close the connection first.
  await expect(page.locator("#status")).toHaveText(/^(Connection lost\.|This view has ended\.)/);
  await expect(page.locator("#mark")).toBeDisabled();await expect(page.locator(".swatch").first()).toBeDisabled();
  const closedArt=await result(workbench.client);await point(page,2,1);expect(await result(workbench.client)).toBe(closedArt);
});

test("an ended workbench is distinguished from newly presented artwork",async({page,workbench})=>{
  const id=await create(workbench.client,"execution-conflict");await show(workbench.client,[id]);await settle(page);await color(page);
  const before=await inspect(workbench.client);
  await page.route("**/api/presentation/action",route=>route.continue({headers:{...route.request().headers(),"x-retro-art-workbench":"old-instance"}}),{times:1});
  await point(page,1,1);
  await expect(page.locator("#status")).toContainText("Ask your agent to reopen");await expect(page.locator("#mark")).toBeDisabled();
  expect((await inspect(workbench.client)).state_id).toBe(before.state_id);
});

test("failed recovery keeps edits disabled until the saved result is confirmed",async({page,workbench})=>{
  const id=await create(workbench.client,"retry-recovery");await show(workbench.client,[id]);await settle(page);await color(page);await point(page,1,1);
  await expect.poll(()=>result(workbench.client)).not.toBe(id);await expect(page.locator("#undo")).toBeEnabled();
  let requests=0,recover=true;
  page.on("request",request=>{if(request.url().endsWith("/api/presentation/action"))requests++;});
  await page.route("**/api/presentation/action",async route=>{await route.fetch();await route.abort();},{times:1});
  await page.route("**/api/presentation/recover",route=>recover?route.abort():route.continue());
  await page.locator("#save").click();await expect.poll(async()=>(await inspect(workbench.client)).saved).not.toBeNull();
  await expect(page.locator("#status")).toHaveClass(/error/);
  await expect(page.locator("#mark")).toBeDisabled();await expect(page.locator("#save")).toBeDisabled();
  await expect(page.locator("#status")).toContainText("Connection lost");
  const saved=(await inspect(workbench.client)).saved;
  expect(await page.locator("canvas").evaluate(c=>Array.from(c.getContext("2d").getImageData(48,48,1,1).data))).toEqual([224,116,64,255]);
  recover=false;await expect(page.locator("#status")).toHaveText("Saved.");await expect(page.locator("#mark")).toBeEnabled();
  expect(requests).toBe(1);expect((await inspect(workbench.client)).saved).toEqual(saved);
});

test("art tool work in another project does not keep an unattended screen alive",async({page,workbench})=>{
  const other=mcpClient(join(workbench.workspace,"other-project"));await other.discover();
  try{
    const id=await create(other,"unrelated-activity");
    await workbench.client.tool("close_workbench",{workbench_id:workbench.client.workbenchId});
    await instance(workbench.client,{idle_timeout_seconds:2});
    for(let i=0;i<6;i++){
      await page.waitForTimeout(500);
      expect((await other.tool("inspect_art",{art_id:id})).structuredContent.ok).toBe(true);
    }
    expect((await workbench.client.tool("inspect_workbench",{})).structuredContent.state).toBe("closed");
  }finally{await stopClient(other);}
});

test("missing or stale instance IDs and foreign origins cannot renew idle time",async({page,workbench})=>{
  const stale=workbench.client.workbenchId;
  await workbench.client.tool("close_workbench",{workbench_id:stale});
  const opened=await instance(workbench.client,{idle_timeout_seconds:2});
  const url=opened.url+"/api/workbench/activity";
  const missing=await page.request.post(url);expect((await missing.json()).error.code).toBe("workbench_conflict");
  const foreign=await page.request.post(url,{headers:{"x-retro-art-workbench":opened.workbench_id,Origin:"https://example.com"}});expect(foreign.status()).toBe(403);
  for(let i=0;i<4;i++){
    const response=await page.request.post(url,{headers:{"x-retro-art-workbench":stale}});
    expect((await response.json()).error.code).toBe("workbench_conflict");
    await page.waitForTimeout(400);
  }
  // Valid renewals during that interval would move shutdown beyond this deadline.
  await expect.poll(async()=>(await workbench.client.tool("inspect_workbench",{})).structuredContent.state,{timeout:1200,intervals:[100]}).toBe("closed");
});


test("an unfinished HTTP body cannot keep a closed workbench alive",async({page,workbench})=>{
  const address=new URL(workbench.url);
  const socket=createConnection({host:address.hostname,port:Number(address.port)});
  socket.on("error",()=>{});
  try{
    await new Promise(resolve=>socket.once("connect",resolve));
    socket.write(`POST /api/presentation/action HTTP/1.1\r\nHost: ${address.host}\r\nContent-Type: application/json\r\nContent-Length: 10000\r\nx-retro-art-workbench: ${workbench.client.workbenchId}\r\n\r\n{`);
    const response=(await workbench.client.tool("close_workbench",{workbench_id:workbench.client.workbenchId})).structuredContent;
    expect(response.state).toBe("closed");expect(await isServing(page,workbench.url)).toBe(false);
    const oldId=workbench.client.workbenchId;const next=await instance(workbench.client);expect(next.workbench_id).not.toBe(oldId);expect(await isServing(page,next.url)).toBe(true);
  }finally{socket.destroy();}
});

test("MCP separates protocol errors, execution errors and typed resource content",async({workbench})=>{
  const client=workbench.client;
  await expect(client.request("tools/call",{name:"does_not_exist",arguments:{}})).rejects.toThrow(/-32602/);
  await expect(client.request("does/not/exist",{})).rejects.toThrow(/-32601/);
  const invalid=await client.tool("open_workbench",{idle_timeout_seconds:0});expect(invalid.isError).toBe(true);expect(invalid.structuredContent.error.code).toBe("invalid_input");expect(JSON.parse(invalid.content[0].text)).toEqual(invalid.structuredContent);
  await expect(client.request("resources/read",{uri:"dotmend://sources/"+"0".repeat(64)})).rejects.toThrow(/-32602/);
  await expect(client.request("tools/list",{cursor:"not-issued"})).rejects.toThrow(/-32602/);
  const id=await create(client,"wire");const rendered=await client.tool("render_art",{art_id:id,scale:1});expect(rendered.isError).toBe(false);expect(JSON.parse(rendered.content[0].text)).toEqual(rendered.structuredContent);
  const image=rendered.content.find(item=>item.type==="image");expect(image.mimeType).toBe("image/png");expect(Buffer.from(image.data,"base64").subarray(0,8)).toEqual(Buffer.from([137,80,78,71,13,10,26,10]));
  const exported=await client.tool("export_art",{art_id:id});const link=exported.content.find(item=>item.type==="resource_link"&&item.uri.endsWith("preview.png"));expect(link.name).toBeTruthy();const resource=await client.request("resources/read",{uri:link.uri});expect(resource.contents[0].uri).toBe(link.uri);expect(resource.contents[0].mimeType).toBe("image/png");const png=Buffer.from(resource.contents[0].blob,"base64");expect(png.subarray(0,8)).toEqual(Buffer.from([137,80,78,71,13,10,26,10]));expect([png.readUInt32BE(16),png.readUInt32BE(20)]).toEqual([8,8]);
  for(const method of ["tools/list","resources/list","resources/templates/list"]){const listing=await client.request(method,{});expect(listing.ttlMs).toBeGreaterThanOrEqual(0);expect(["private","public"]).toContain(listing.cacheScope);await expect(client.request(method,{cursor:"not-issued"})).rejects.toThrow(/-32602/);}
  const guide=await client.request("resources/read",{uri:"dotmend://guides/editing"});expect(guide.ttlMs).toBeGreaterThanOrEqual(0);expect(["private","public"]).toContain(guide.cacheScope);expect(guide.contents[0].mimeType).toBe("text/markdown");expect(guide.contents[0].text).toContain("open_workbench");
  const previousGuide=await client.request("resources/read",{uri:"retro-art://guides/editing"});expect(previousGuide.contents[0].text).toBe(guide.contents[0].text);
  const previousExport=await client.request("resources/read",{uri:link.uri.replace("dotmend://","retro-art://")});expect(previousExport.contents[0].blob).toBe(resource.contents[0].blob);
});


test("agent guidance and errors are English while caller text is preserved",async({workbench})=>{
  const client=mcpClient(workbench.workspace);
  try{
    const info=await client.discover();expect(info.instructions).toMatch(/^[\x00-\x7F]+$/);expect(info.instructions).toContain("dotmend://guides/editing");
    const catalog=await client.request("tools/list",{});for(const tool of catalog.tools)expect(tool.description).toMatch(/^[\x00-\x7F]+$/);
    const guide=await client.request("resources/read",{uri:"dotmend://guides/editing"});expect(guide.contents[0].text).toMatch(/^[\x00-\x7F]+$/);
    const id=await create(client,"Renée’s original");
    for(const [name,args,code] of [["open_workbench",{},"workbench_busy"],["inspect_art",{art_id:"missing"},"art_not_found"],["render_art",{art_id:id,scale:0},"limit_exceeded"],["request_edit",{base_art_id:id,write_region:{x:0,y:0,width:8,height:8},instruction:""},"invalid_input"]]){
      const response=await client.tool(name,args);expect(response.isError).toBe(true);expect(response.structuredContent.error.code).toBe(code);expect(response.structuredContent.error.message).toMatch(/^[\x00-\x7F]+$/);expect(JSON.parse(response.content[0].text)).toEqual(response.structuredContent);
    }
    const instruction="Refine Renée’s hair — preserve the face";const request=(await client.tool("request_edit",{base_art_id:id,write_region:{x:0,y:0,width:8,height:8},instruction})).structuredContent;
    const read=(await client.tool("inspect_edit_request",{request_id:request.request_id})).structuredContent;
    expect(JSON.stringify(read)).toContain(instruction);expect(JSON.stringify(read)).toContain("Renée’s original");
  }finally{await stopClient(client);}
});

test("MCP request metadata selects discovery and version-specific resource errors",async({workbench})=>{
  const client=mcpClient(workbench.workspace,join(workbench.workspace,"runtime"),true);
  try{
    const info=await client.request("server/discover",{});expect(info.instructions).toContain("open_workbench");expect(info.capabilities.tools).toBeDefined();expect(info.supportedVersions).toEqual(["2026-07-28"]);
    const catalog=await client.request("tools/list",{});expect(catalog.tools.some(tool=>tool.name==="open_workbench")).toBe(true);
    const response=await client.tool("inspect_workbench",{});expect(response.resultType).toBe("complete");expect(response.structuredContent.state).toBe("busy");expect(JSON.parse(response.content[0].text)).toEqual(response.structuredContent);
    await expect(client.request("resources/read",{uri:"dotmend://sources/"+"0".repeat(64)})).rejects.toThrow(/-32602/);
    const guide=await client.request("resources/read",{uri:"dotmend://guides/editing"});expect(guide.resultType).toBe("complete");expect(guide.contents[0].mimeType).toBe("text/markdown");
  }finally{await stopClient(client);}
});

test("excess MCP calls fail with a retry delay and the same connection recovers",async({workbench})=>{
  const replies=await Promise.all(Array.from({length:256},()=>workbench.client.tool("inspect_presentation",{})));
  const limited=replies.filter(reply=>reply.isError);expect(limited.length).toBeGreaterThan(0);
  for(const reply of limited){expect(reply.structuredContent.error.code).toBe("rate_limited");expect(reply.structuredContent.error.details.retry_after_ms).toBeGreaterThan(0);expect(JSON.parse(reply.content[0].text)).toEqual(reply.structuredContent);}
  await expect.poll(async()=> (await workbench.client.tool("inspect_presentation",{})).isError).toBe(false);
});


test("explicit control works across MCP connections and separates tasks on one connection",async({page,workbench})=>{
  const relay=mcpClient(workbench.workspace);
  const shared={control_id:workbench.client.controlId};
  try{
    // No discover or initialize request is required before tools/call.
    const remote=(await relay.tool("open_workbench",shared)).structuredContent;expect(remote.instance.workbench_id).toBe(workbench.client.workbenchId);expect(remote.instance.url).toBe(workbench.url);
    const unrelated={control_id:randomUUID()};expect((await workbench.client.tool("open_workbench",unrelated)).structuredContent.error.code).toBe("workbench_busy");expect((await workbench.client.tool("inspect_workbench",unrelated)).structuredContent.state).toBe("busy");
    const id=await create(relay,"forwarded");const previous=await inspect(relay);
    const presented=(await relay.tool("present_art",{...shared,workbench_id:remote.instance.workbench_id,view:{title:"Prepared through another connection",items:[{kind:"art",art_id:id,label:"Image to edit",region:{x:0,y:0,width:8,height:8},scale:32,editable:true}],expected_presentation_id:previous?.presentation_id??null,expected_state_id:previous?.state_id??null}})).structuredContent;expect(presented.ok).toBe(true);await settle(page,"Prepared through another connection");
    const missing=relay.request("tools/list",{},false);await expect(missing).rejects.toThrow(/-32602/);
    const catalog=await relay.request("tools/list",{});expect(catalog.resultType).toBe("complete");
    const closed=(await relay.tool("close_workbench",{...shared,workbench_id:remote.instance.workbench_id})).structuredContent;expect(closed.state).toBe("closed");expect(await isServing(page,workbench.url)).toBe(false);
    expect((await inspect(relay)).state_id).toBe(presented.presentation.state_id);
  }finally{await stopClient(relay);}
});


test("legacy initialization is rejected instead of silently negotiating a stateful protocol",async({workbench})=>{
  const client=mcpClient(workbench.workspace,join(workbench.workspace,"runtime"),false);
  try{await expect(client.request("initialize",{protocolVersion:"2025-11-25",capabilities:{},clientInfo:{name:"legacy-check",version:"test"}})).rejects.toThrow(/-32022/);}
  finally{await stopClient(client);}
});

async function stroke(page,canvas,from,to,button="left"){
  await expect(page.locator("#mark")).toBeEnabled();
  const box=await canvas.boundingBox();
  const start={x:box.x+from[0]*32+16,y:box.y+from[1]*32+16};
  const end={x:box.x+to[0]*32+16,y:box.y+to[1]*32+16};
  await page.mouse.move(start.x,start.y);await page.mouse.down({button});
  await page.mouse.move(end.x,end.y,{steps:Math.max(Math.abs(to[0]-from[0]),Math.abs(to[1]-from[1]),1)});
  await page.mouse.up({button});
}
async function marks(client){return (await inspect(client)).state.concerns||[];}

test("the human saves several candidates and the agent continues from those exact choices",async({page,workbench},testInfo)=>{
  const base=await create(workbench.client,"candidate-options");
  const ids=[];
  for(const x of [1,3,5])ids.push((await workbench.client.tool("edit_art",{art_id:base,write_region:{x:0,y:0,width:8,height:8},operations:[{kind:"fill_rect",rect:{x,y:1,width:2,height:6},index:2}]})).structuredContent.art_id);
  const items=[base,...ids].map((art_id,index)=>({kind:"art",art_id,label:index?`Option ${index}`:"Original",region:{x:0,y:0,width:8,height:8},scale:20,editable:false}));
  const initial=await show(workbench.client,ids,{title:"Choose directions to explore",items,candidate_choices:{item_indices:[1,2,3]}});
  await expect(page.locator("#title")).toHaveText("Choose directions to explore");
  await expect(page.getByRole("button",{name:"Choose Original",exact:true})).toHaveCount(0);
  const first=page.getByRole("button",{name:"Choose Option 1",exact:true});
  const second=page.getByRole("button",{name:"Choose Option 2",exact:true});
  const third=page.getByRole("button",{name:"Choose Option 3",exact:true});
  await first.click();await expect(first).toHaveAttribute("aria-pressed","true");
  await third.focus();await page.keyboard.press("Space");await expect(third).toHaveAttribute("aria-pressed","true");await expect(third).toBeFocused();
  await expect(second).toHaveAttribute("aria-pressed","false");
  const chosen=[{item_index:1,art_id:ids[0]},{item_index:3,art_id:ids[2]}];
  await expect.poll(async()=>(await inspect(workbench.client)).state.chosen_candidates).toEqual(chosen);
  await page.keyboard.press(`${modifier}+s`);await expect(page.locator("#status")).toHaveText("Saved.");
  const saved=(await inspect(workbench.client)).saved;expect(saved.chosen_candidates).toEqual(chosen);expect(saved.art_ids).toEqual([base,...ids]);
  await page.reload();await expect(first).toHaveAttribute("aria-pressed","true");await expect(third).toHaveAttribute("aria-pressed","true");
  await first.click();await expect(first).toHaveAttribute("aria-pressed","false");expect((await inspect(workbench.client)).saved).toEqual(saved);
  await first.click();await expect(first).toHaveAttribute("aria-pressed","true");
  await page.screenshot({path:testInfo.outputPath("candidate-choices.png"),fullPage:true});
  const next=[];
  for(const candidate of saved.chosen_candidates){
    const output=(await workbench.client.tool("edit_art",{art_id:candidate.art_id,write_region:{x:0,y:0,width:8,height:8},operations:[{kind:"set_pixels",pixels:[{x:0,y:0,index:2}]}]})).structuredContent;
    expect(output.ok).toBe(true);expect(output.base_art_id).toBe(candidate.art_id);next.push(output.art_id);
  }
  await show(workbench.client,next,{title:"Refined choices",items:next.map((art_id,i)=>({kind:"art",art_id,label:`Refinement ${i+1}`,region:{x:0,y:0,width:8,height:8},scale:20,editable:false})),candidate_choices:{item_indices:[0,1]}});
  await expect(page.locator("#title")).toHaveText("Refined choices");await expect(page.locator('.candidate-choice[aria-pressed="true"]')).toHaveCount(0);
  const history=(await workbench.client.tool("inspect_presentation",{presentation_id:initial.presentation_id,state_id:saved.state_id})).structuredContent.presentation;
  expect(history.state.chosen_candidates).toEqual(chosen);
});

test("lost choice responses and a stale click preserve the right candidates",async({page,workbench})=>{
  const ids=[await create(workbench.client,"choice-a"),await create(workbench.client,"choice-b")];
  const options={items:ids.map((art_id,i)=>({kind:"art",art_id,label:`Option ${i+1}`,region:{x:0,y:0,width:8,height:8},scale:16,editable:false})),candidate_choices:{item_indices:[0,1]}};
  await show(workbench.client,ids,options);
  const button=page.getByRole("button",{name:"Choose Option 1",exact:true});await expect(button).toBeVisible();
  await page.route("**/api/presentation/action",async route=>{await route.fetch();await route.abort();},{times:1});
  await button.click();await expect(button).toHaveAttribute("aria-pressed","true");
  expect((await inspect(workbench.client)).state.chosen_candidates).toEqual([{item_index:0,art_id:ids[0]}]);
  let release,reached;const blocked=new Promise(resolve=>release=resolve),entered=new Promise(resolve=>reached=resolve);
  await page.route("**/api/presentation/action",async route=>{reached();await blocked;await route.continue();},{times:1});
  await page.getByRole("button",{name:"Choose Option 2",exact:true}).click();await entered;
  const next=await show(workbench.client,ids,{...options,title:"A new comparison"});
  release();await expect(page.locator("#title")).toHaveText("A new comparison");await expect(page.locator('.candidate-choice[aria-pressed="true"]')).toHaveCount(0);
  const current=await inspect(workbench.client);expect(current.state_id).toBe(next.state_id);expect(current.state.chosen_candidates||[]).toEqual([]);
});

test("marking an overflowing picture preserves its scroll position through undo and save",async({page,workbench})=>{
  await page.setViewportSize({width:700,height:700});
  const id=(await workbench.client.tool("create_art",{target:{resource_id:"scrolling-art",width:64,height:64,palette:["#000000","#E07440"],transparent_index:0,allowed_indices:[0,1],constraints_ref:null,requirements:[]},initial:{kind:"fill",index:1}})).structuredContent.art_id;
  await show(workbench.client,[id],{items:[{kind:"art",art_id:id,label:"Large picture",region:{x:0,y:0,width:64,height:64},scale:16,editable:true}]});await settle(page);
  await page.locator("#mark").click();
  await page.locator(".drawing").evaluate(well=>{well.scrollLeft=160;window.scrollTo(0,300);});
  const scroll=()=>page.locator(".drawing").evaluate(well=>({left:well.scrollLeft,top:well.scrollTop,pageY:window.scrollY}));
  const before=await scroll();expect(before.left).toBeGreaterThan(0);expect(before.pageY).toBeGreaterThan(0);
  const box=await page.locator(".drawing").boundingBox();
  await page.mouse.move(box.x+50,300);await page.mouse.down();await page.mouse.move(box.x+220,350,{steps:12});await page.mouse.up();
  await expect.poll(async()=>(await marks(workbench.client))[0]?.pixels.length||0).toBeGreaterThan(1);
  await expect(page.locator("#undo")).toBeEnabled();
  expect(await scroll()).toEqual(before);
  await page.keyboard.press(`${modifier}+z`);await expect.poll(()=>marks(workbench.client)).toEqual([]);await expect(page.locator("#save")).toBeEnabled();await expect(page.locator("#undo")).toBeDisabled();expect(await scroll()).toEqual(before);
  await page.keyboard.press(`${modifier}+s`);await expect(page.locator("#status")).toHaveText("Saved.");expect(await scroll()).toEqual(before);
});

test("issue mode adds and removes exact pixels without painting and the agent reads the saved marks",async({page,workbench},testInfo)=>{
  const id=await create(workbench.client,"marked-crop");
  await show(workbench.client,[id],{items:[{kind:"art",art_id:id,label:"Hair detail",region:{x:2,y:3,width:4,height:3},scale:32,editable:true}]});await settle(page);
  const canvas=page.locator("canvas.markable");
  await color(page);await page.locator("#mark").click();await expect(page.locator("#mark")).toHaveAttribute("aria-pressed","true");
  await expect(page.locator('.swatch[aria-pressed="true"]')).toHaveCount(0);
  await page.evaluate(()=>{window.contextPrevented=[];document.addEventListener("contextmenu",e=>window.contextPrevented.push(e.defaultPrevented));});
  await stroke(page,canvas,[0,0],[3,1]);
  const diagonal=[{x:2,y:3},{x:3,y:3},{x:4,y:4},{x:5,y:4}];
  await expect.poll(async()=>(await marks(workbench.client))[0]?.pixels).toEqual(diagonal);
  await stroke(page,canvas,[0,0],[1,0],"right");
  await expect.poll(async()=>(await marks(workbench.client))[0]?.pixels).toEqual(diagonal.slice(2));
  await stroke(page,canvas,[2,2],[2,2]);
  await expect.poll(async()=>(await marks(workbench.client))[0]?.pixels).toEqual([...diagonal.slice(2),{x:4,y:5}]);
  await stroke(page,canvas,[2,2],[2,2],"right");
  await expect.poll(async()=>(await marks(workbench.client))[0]?.pixels).toEqual(diagonal.slice(2));
  const context=await page.evaluate(()=>window.contextPrevented);expect(context.length).toBeGreaterThan(0);expect(context.every(Boolean)).toBe(true);
  expect(await result(workbench.client)).toBe(id);expect((await rows(workbench.client,id)).flat().every(i=>i===0)).toBe(true);
  await expect(page.locator("#undo")).toBeEnabled();await page.keyboard.press(`${modifier}+z`);
  await expect.poll(async()=>(await marks(workbench.client))[0]?.pixels).toEqual([...diagonal.slice(2),{x:4,y:5}]);
  await expect(page.locator("#undo")).toBeDisabled();
  await expect(page.locator("#save")).toBeEnabled();await page.keyboard.press(`${modifier}+s`);await expect(page.locator("#status")).toHaveText("Saved.");
  const saved=(await inspect(workbench.client)).saved;
  expect(saved.concerns[0]).toEqual({item_index:0,art_id:id,bounds:{x:4,y:4,width:2,height:2},pixels:[...diagonal.slice(2),{x:4,y:5}]});
  const focused=await workbench.client.tool("focus_art",{art_id:saved.concerns[0].art_id,region:saved.concerns[0].bounds,context_padding:1,scale:4});expect(focused.structuredContent.ok).toBe(true);
  await page.reload();await settle(page);await expect(page.locator("#save")).toBeDisabled();expect((await inspect(workbench.client)).saved).toEqual(saved);
  await page.locator("#mark").click();await page.screenshot({path:testInfo.outputPath("marked-pixels.png"),fullPage:true});
  await color(page);await expect(page.locator("#mark")).toHaveAttribute("aria-pressed","false");
  await canvas.click({position:{x:16,y:16}});await expect.poll(()=>result(workbench.client)).not.toBe(id);
  const after=await inspect(workbench.client);expect(after.state.concerns[0].pixels).toEqual(saved.concerns[0].pixels);expect(after.state.concerns[0].art_id).toBe(after.state.art_ids[0]);expect(after.saved).toEqual(saved);
});

test("protected and readonly art can be marked but references and playback cannot",async({page,workbench})=>{
  const id=await create(workbench.client,"protected-marks");
  const selection=(await workbench.client.tool("create_selection",{art_id:id,region:{x:0,y:0,width:8,height:8},selector:{kind:"rect"}})).structuredContent;
  const request=(await workbench.client.tool("request_edit",{base_art_id:id,write_region:{x:0,y:0,width:8,height:8},instruction:"Preserve every pixel",protected_selection_ids:[selection.selection_id]})).structuredContent;
  await show(workbench.client,[id],{items:[{kind:"art",art_id:id,label:"Protected art",region:{x:0,y:0,width:8,height:8},scale:32,editable:true,request_id:request.request_id}]});await settle(page);
  await page.locator("#mark").click();await stroke(page,page.locator("canvas.markable"),[1,1],[1,1]);await expect.poll(async()=>(await marks(workbench.client))[0]?.pixels).toEqual([{x:1,y:1}]);
  expect(await result(workbench.client)).toBe(id);
  await show(workbench.client,[id],{title:"Inspect only",items:[{kind:"art",art_id:id,label:"Read-only art",region:{x:0,y:0,width:8,height:8},scale:32,editable:false}]});
  await expect(page.locator("#title")).toHaveText("Inspect only");await expect(page.locator(".palette")).toHaveCount(0);
  await page.locator("#mark").click();await stroke(page,page.locator("canvas.markable"),[2,2],[2,2]);await expect.poll(async()=>(await marks(workbench.client))[0]?.pixels).toEqual([{x:2,y:2}]);
  await page.locator("#save").click();await expect(page.locator("#status")).toHaveText("Saved.");await expect(page.locator("#save")).toBeDisabled();expect((await inspect(workbench.client)).saved.concerns[0].art_id).toBe(id);
  const rendered=await workbench.client.tool("render_art",{art_id:id,scale:1});await writeFile(join(workbench.workspace,"mark-reference.png"),Buffer.from(rendered.content.find(c=>c.type==="image").data,"base64"));
  const attached=(await workbench.client.tool("attach_reference",{art_id:id,source_path:"mark-reference.png",label:"Reference",role:"reference"})).structuredContent.art_id;
  const inspected=(await workbench.client.tool("inspect_art",{art_id:attached})).structuredContent;
  for(const item of [{kind:"reference",art_id:attached,source_hash:inspected.references[0].source_hash,label:"Reference",region:{x:0,y:0,width:8,height:8},scale:32},{kind:"art",art_id:id,label:"Animation",region:{x:0,y:0,width:8,height:8},scale:32,editable:false,playback:[{art_id:id,label:"Idle",duration_ms:100}]}]){
    await show(workbench.client,[id],{title:item.label,items:[item]});await expect(page.locator("#title")).toHaveText(item.label);await expect(page.locator("#mark")).toBeDisabled();
    const view=await inspect(workbench.client);
    const response=await page.request.post(workbench.url+"/api/presentation/action",{headers:{"x-retro-art-workbench":workbench.client.workbenchId},data:{action:"mark",presentation_id:view.presentation_id,expected_state_id:view.state_id,item_index:0,pixels:[{x:1,y:1}],marked:true}});
    expect(response.ok()).toBe(false);expect((await response.json()).error.code).toBe("invalid_input");expect((await inspect(workbench.client)).state_id).toBe(view.state_id);
  }
});

test("lost marking responses and late strokes preserve the right presentation",async({page,workbench})=>{
  const a=await create(workbench.client,"mark-a"),b=await create(workbench.client,"mark-b");await show(workbench.client,[a]);await settle(page);await page.locator("#mark").click();
  await page.route("**/api/presentation/action",async route=>{await route.fetch();await route.abort();},{times:1});
  await stroke(page,page.locator("canvas.markable"),[1,1],[1,1]);
  await expect(page.locator("#status")).toHaveText("Click Save when you are ready.");
  await expect.poll(async()=>(await marks(workbench.client))[0]?.pixels).toEqual([{x:1,y:1}]);
  const original=await inspect(workbench.client);
  let release,reached;const blocked=new Promise(r=>release=r),entered=new Promise(r=>reached=r);
  await page.route("**/api/presentation/action",async route=>{reached();await blocked;await route.continue();},{times:1});
  await stroke(page,page.locator("canvas.markable"),[2,2],[2,2]);await entered;
  await show(workbench.client,[b],{title:"Next candidate"});release();await settle(page,"Next candidate");
  await expect(page.locator("#status")).toContainText("New artwork has arrived");expect(await marks(workbench.client)).toEqual([]);
  const archived=(await workbench.client.tool("inspect_presentation",{presentation_id:original.presentation_id})).structuredContent.presentation;
  expect(archived.state.concerns).toEqual(original.state.concerns);expect(archived.state.art_ids[0]).toBe(a);expect(await result(workbench.client)).toBe(b);
});

test("canceling a marking stroke removes its preview without storing pixels",async({page,workbench})=>{
  const id=await create(workbench.client,"cancel-mark");await show(workbench.client,[id]);await settle(page);await page.locator("#mark").click();
  const canvas=page.locator("canvas.markable"),box=await canvas.boundingBox();
  const before=await canvas.evaluate(c=>Array.from(c.getContext("2d").getImageData(48,48,1,1).data));
  await canvas.evaluate(c=>c.addEventListener("pointerdown",e=>c.dataset.pointer=String(e.pointerId),{once:true}));
  await page.mouse.move(box.x+48,box.y+48);await page.mouse.down();
  expect(await canvas.evaluate(c=>Array.from(c.getContext("2d").getImageData(48,48,1,1).data))).not.toEqual(before);
  await canvas.evaluate(c=>c.dispatchEvent(new PointerEvent("pointercancel",{pointerId:Number(c.dataset.pointer),bubbles:true})));await page.mouse.up();
  await expect(page.locator("#mark")).toBeEnabled();expect(await marks(workbench.client)).toEqual([]);
  expect(await canvas.evaluate(c=>Array.from(c.getContext("2d").getImageData(48,48,1,1).data))).toEqual(before);expect(await result(workbench.client)).toBe(id);
});

test("large palettes prioritize issue marks and keep twenty stable original indices",async({page,workbench})=>{
  const client=workbench.client;
  const palette=Array.from({length:32},(_,i)=>`#${(i*8).toString(16).padStart(2,"0").repeat(3)}`);
  palette[31]=palette[30];
  const pixels=[...Array(32).fill(31),...Array(9).fill(30),...Array.from({length:23},(_,i)=>i+1)];
  const created=(await client.tool("create_art",{target:{resource_id:"many-colors",width:8,height:8,palette,transparent_index:0,allowed_indices:Array.from({length:32},(_,i)=>i),constraints_ref:null,requirements:[]},initial:{kind:"indices",rows:Array.from({length:8},(_,y)=>pixels.slice(y*8,y*8+8))}})).structuredContent;
  expect(created.ok).toBe(true);
  await show(client,[created.art_id]);await settle(page);
  const choices=()=>page.locator('.palette .swatch').evaluateAll(nodes=>nodes.map(n=>Number(n.dataset.index)));
  const expected=[0,31,30,...Array.from({length:17},(_,i)=>i+1)];
  expect(await choices()).toEqual(expected);
  await expect(page.locator('#mark')).toHaveAttribute('aria-pressed','true');
  await expect(page.getByText('Showing up to 20 common colors. Mark areas to change and ask your agent to edit them.')).toBeVisible();
  await point(page,1,1);
  await expect.poll(async()=>(await inspect(client)).state.concerns?.[0]?.pixels).toEqual([{x:1,y:1}]);
  expect(await result(client)).toBe(created.art_id);
  await page.locator('.palette [data-index="17"]').click();
  await expect(page.locator('#mark')).toHaveAttribute('aria-pressed','false');
  for(let x=0;x<8;x++){await point(page,x,0);await expect.poll(async()=>(await rows(client,await result(client)))[0][x]).toBe(17);}
  expect(await choices()).toEqual(expected);
  const modified=(await client.tool('inspect_art',{art_id:await result(client),include_indices:true})).structuredContent;
  expect(modified.target.palette).toEqual(palette);
  expect(modified.target.allowed_indices).toEqual(Array.from({length:32},(_,i)=>i));
  await page.locator('#undo').click();await expect.poll(async()=>(await rows(client,await result(client)))[0][7]).toBe(31);
  expect(await choices()).toEqual(expected);
  await page.locator('#save').click();await expect(page.locator('#status')).toHaveText('Saved.');
  expect(await choices()).toEqual(expected);
  await show(client,[created.art_id],{items:[{kind:'art',art_id:created.art_id,label:'Small crop',region:{x:1,y:5,width:7,height:1},scale:32,editable:true}]});
  await expect.poll(async()=>(await choices()).slice(0,8)).toEqual([0,1,2,3,4,5,6,7]);
});
