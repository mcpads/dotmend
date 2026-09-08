import { test as base, expect } from "@playwright/test";
import { spawn } from "node:child_process";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { createInterface } from "node:readline";
import { randomUUID } from "node:crypto";
import { createConnection } from "node:net";

const binary = resolve("target/debug/dotmend");
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
  await expect(page.locator("#status")).toContainText("Please check your saved work");
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
  await expect.poll(async()=>(await workbench.client.tool("inspect_workbench",{})).structuredContent.state,{timeout:7000}).toBe("closed");
  expect(await isServing(page,short.url)).toBe(false);
  const renewed=await instance(workbench.client);expect(renewed.workbench_id).not.toBe(short.workbench_id);expect((await inspect(workbench.client)).state_id).toBe(draft.state_id);
  expect((await workbench.client.tool("close_workbench",{workbench_id:short.workbench_id})).structuredContent.error.code).toBe("workbench_conflict");
  expect(await isServing(page,renewed.url)).toBe(true);
});

test("HTTP cannot bypass managed MCP ownership for tool mutations",async({page,workbench})=>{
  const id=await create(workbench.client,"http");await show(workbench.client,[id]);const view=await inspect(workbench.client);
  const response=await page.request.post(workbench.url+"/api/call",{data:{tool:"present_art",arguments:{workbench_id:workbench.client.workbenchId,view:view.presentation}}});
  expect((await response.json()).error.code).toBe("mcp_required");expect((await inspect(workbench.client)).state_id).toBe(view.state_id);
  const noIdentity=await page.request.post(workbench.url+"/api/presentation/action",{data:{action:"save",presentation_id:view.presentation_id,expected_state_id:view.state_id}});
  expect((await noIdentity.json()).error.code).toBe("workbench_conflict");
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
  await expect(page.locator("#status")).toContainText("Please check your saved work");
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
