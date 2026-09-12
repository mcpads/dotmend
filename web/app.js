"use strict";
const $=id=>document.getElementById(id);
const state={workbenchId:null,connection:"checking",pending:null,activityInterval:30000,lastActivity:-Infinity,renewing:false,view:null,arts:new Map(),sources:new Map(),colors:new Map(),paletteChoices:new Map(),marking:false,stroke:null,busy:false,refresh:0,polling:null,timers:new Set()};
const MAX_VISIBLE_COLORS=20;
const REQUEST_TIMEOUT=5000;
function visibleColors(art,region){
  const allowed=art.target.allowed_indices;
  if(allowed.length<=MAX_VISIBLE_COLORS)return [...allowed];
  const counts=new Map(allowed.map(index=>[index,0]));
  for(let y=region.y;y<region.y+region.height;y++)for(let x=region.x;x<region.x+region.width;x++){
    const index=art.indices[y][x];if(counts.has(index))counts.set(index,counts.get(index)+1);
  }
  const transparent=art.target.transparent_index;
  const erase=allowed.includes(transparent)?[transparent]:[];
  const ranked=allowed.filter(index=>index!==transparent).sort((a,b)=>counts.get(b)-counts.get(a));
  return [...erase,...ranked.slice(0,MAX_VISIBLE_COLORS-erase.length)];
}
function status(text,error=false){$("status").textContent=text;$("status").classList.toggle("error",error);}
async function deadline(operation,cancel=()=>{}){
  let timer;
  try{return await Promise.race([operation,new Promise((_,reject)=>{timer=setTimeout(()=>{const error=new Error("The response timed out.");error.code="request_timeout";reject(error);cancel();},REQUEST_TIMEOUT);})]);}
  finally{clearTimeout(timer);}
}
async function json(url,options){
  const controller=new AbortController();
  return deadline((async()=>{
    const response=await fetch(url,{cache:"no-store",...options,signal:controller.signal,headers:{...options?.headers,...(state.workbenchId?{"x-retro-art-workbench":state.workbenchId}:{})}});
    const data=await response.json();
    if(!response.ok||data.ok===false){const error=new Error(data.error?.message||"Please check your connection.");error.code=data.error?.code;throw error;}
    return data;
  })(),()=>controller.abort());
}
function canAct(){return state.connection==="ready"&&!state.pending&&!state.busy&&!state.stroke;}
function connectionFailed(error){
  state.connection=error.code==="workbench_conflict"?"closed":"offline";controls();
  status(state.connection==="closed"?"This view has ended. Ask your agent to reopen it.":state.pending?"Connection lost. Checking your last change before you continue.":"Connection lost. Checking your saved work.",true);
}

async function renewActivity(event){
  if(state.connection!=="ready"||!event.isTrusted||document.visibilityState!=="visible"||!state.workbenchId||state.renewing||performance.now()-state.lastActivity<state.activityInterval)return;
  state.lastActivity=performance.now();state.renewing=true;
  try{await json("/api/workbench/activity",{method:"POST"});}catch{/* Synchronization and saved actions report connection failures. */}finally{state.renewing=false;}
}
function loadArt(id){if(!state.arts.has(id))state.arts.set(id,json(`/api/art/${id}`).catch(error=>{state.arts.delete(id);throw error;}));return state.arts.get(id);}
function loadImage(src){const img=new Image();return deadline(new Promise((resolve,reject)=>{img.onload=()=>resolve(img);img.onerror=()=>reject(new Error("Could not load the image."));img.src=src;}),()=>{img.src="";});}
function loadSource(hash){if(!state.sources.has(hash))state.sources.set(hash,loadImage(`/api/source/${hash}`).catch(error=>{state.sources.delete(hash);throw error;}));return state.sources.get(hash);}
async function loadAnimation(item){const result=await json("/api/call",{method:"POST",headers:{"Content-Type":"application/json"},body:JSON.stringify({tool:"render_art_set",arguments:{frames:item.playback,scale:item.scale,view:"frames"}})});result.loaded=await Promise.all(result.images.map(loadImage));return result;}

function stopAnimations(){for(const timer of state.timers)clearTimeout(timer);state.timers.clear();}
function later(fn,delay){const timer=setTimeout(()=>{state.timers.delete(timer);fn();},Math.min(delay,60000));state.timers.add(timer);}
function animate(canvas,result){let index=0;const token=state.refresh;canvas.width=result.display_width;canvas.height=result.display_height;const ctx=canvas.getContext("2d");ctx.imageSmoothingEnabled=false;function frame(){if(token!==state.refresh)return;ctx.clearRect(0,0,canvas.width,canvas.height);checker(ctx,Math.ceil(canvas.width/8),Math.ceil(canvas.height/8),8);ctx.drawImage(result.loaded[result.frames[index].image_index],0,0);const end=performance.now()+result.frames[index].duration_ms;function wait(){if(token!==state.refresh)return;const remaining=end-performance.now();if(remaining>0)later(wait,remaining);else{index=(index+1)%result.frames.length;frame();}}later(wait,result.frames[index].duration_ms);}frame();}
function editable(){return state.view?.presentation.items.some(i=>i.kind==="art"&&i.editable);}
function choiceIndices(){return state.view?.presentation.candidate_choices?.item_indices||[];}
function chosenIndices(){return (state.view?.state.chosen_candidates||[]).map(candidate=>candidate.item_index);}
function choiceHint(){return "Choose any candidates you want to explore, then Save to share your choice. Click a chosen candidate again to remove it.";}
function markable(item){return item.kind==="art"&&!item.playback;}
function canMark(){return state.view?.presentation.items.some(markable);}
function controls(){
  const working=!canAct();
  $("undo").disabled=working||!state.view?.undo_available;
  $("save").disabled=working||!(editable()||canMark()||choiceIndices().length)||!state.view?.dirty;
  $("mark").disabled=working||!canMark();$("mark").setAttribute("aria-pressed",String(state.marking));
  $("hint").hidden=!(editable()||canMark()||choiceIndices().length);
  $("hint").textContent=state.marking?(editable()?"Left-click or drag to mark pixels. Right-click or drag to unmark. Choose a palette color to paint.":"Left-click or drag to mark pixels. Right-click or drag to unmark."):choiceIndices().length?choiceHint():editable()?"Choose a color, then click the pixels you want to change. Use Mark issues to point out an area.":"Use Mark issues to point out pixels for your agent.";
  const chosen=chosenIndices();
  for(const button of document.querySelectorAll(".candidate-choice")){
    const selected=chosen.includes(Number(button.dataset.item));
    button.disabled=working;button.setAttribute("aria-pressed",String(selected));button.textContent=selected?"Chosen":"Choose";
    button.closest(".picture").classList.toggle("chosen",selected);
  }
  for(const canvas of document.querySelectorAll("canvas.markable"))canvas.classList.toggle("marking",state.marking);
  for(const swatch of document.querySelectorAll(".swatch")){
    swatch.disabled=working;
    swatch.setAttribute("aria-pressed",String(!state.marking&&state.colors.get(Number(swatch.dataset.item))===Number(swatch.dataset.index)));
  }
}
function checker(ctx,width,height,scale){for(let y=0;y<height;y++)for(let x=0;x<width;x++){ctx.fillStyle=(x+y)%2?"#d2d7cf":"#edf0e9";ctx.fillRect(x*scale,y*scale,scale,scale);}}
function draw(canvas,item,art,itemIndex,stroke){
  const ctx=canvas.getContext("2d"),r=item.region,s=item.scale;
  const pending=stroke?.action==="paint"?stroke.pixels:null;
  ctx.imageSmoothingEnabled=false;checker(ctx,r.width,r.height,s);
  for(let y=0;y<r.height;y++)for(let x=0;x<r.width;x++){
    const index=pending?.get(`${x+r.x},${y+r.y}`)?.index??art.indices[y+r.y][x+r.x];
    if(index!==art.target.transparent_index){ctx.fillStyle=art.target.palette[index];ctx.fillRect(x*s,y*s,s,s);}
  }
  const marked=new Map((state.view?.state.concerns?.find(c=>c.item_index===itemIndex)?.pixels||[]).map(p=>[`${p.x},${p.y}`,p]));
  if(stroke?.action==="mark")for(const [key,pixel] of stroke.pixels){if(stroke.marked)marked.set(key,pixel);else marked.delete(key);}
  ctx.save();ctx.fillStyle="rgba(255,180,0,.28)";ctx.strokeStyle="#995400";ctx.lineWidth=Math.max(1,Math.min(2,s/8));
  for(const p of marked.values()){
    const x=(p.x-r.x)*s,y=(p.y-r.y)*s;
    ctx.fillRect(x,y,s,s);ctx.strokeRect(x+.5,y+.5,s-1,s-1);
    if(s>=8){ctx.beginPath();ctx.moveTo(x+2,y+s-2);ctx.lineTo(x+s-2,y+2);ctx.stroke();}
  }
  ctx.restore();
}
function coordinate(event,canvas,item){const box=canvas.getBoundingClientRect(),r=item.region;return{x:r.x+Math.max(0,Math.min(r.width-1,Math.floor((event.clientX-box.left)*r.width/box.width))),y:r.y+Math.max(0,Math.min(r.height-1,Math.floor((event.clientY-box.top)*r.height/box.height)))};}
function traceStroke(event,canvas,item){
  const stroke=state.stroke;if(!stroke||stroke.pointerId!==event.pointerId)return;
  const p=coordinate(event,canvas,item),start=stroke.last||p,steps=Math.max(Math.abs(p.x-start.x),Math.abs(p.y-start.y),1);
  for(let i=0;i<=steps;i++){
    const x=Math.round(start.x+(p.x-start.x)*i/steps),y=Math.round(start.y+(p.y-start.y)*i/steps);
    stroke.pixels.set(`${x},${y}`,stroke.action==="paint"?{x,y,index:stroke.color}:{x,y});
  }
  stroke.last=p;draw(canvas,item,stroke.art,stroke.index,stroke);
}
function attachBrush(canvas,item,index,art){
  canvas.oncontextmenu=event=>{if(state.marking)event.preventDefault();};
  canvas.onpointerdown=event=>{
    if(!canAct()||!(state.marking?[0,2].includes(event.button):item.editable&&event.button===0))return;
    const color=state.colors.get(index);
    if(!state.marking&&color===undefined){status("Choose a color below first.");return;}
    event.preventDefault();canvas.setPointerCapture(event.pointerId);
    state.stroke={canvas,item,index,art,pixels:new Map(),color,view:state.view,pointerId:event.pointerId,action:state.marking?"mark":"paint",marked:event.button===0};
    traceStroke(event,canvas,item);controls();
  };
  canvas.onpointermove=event=>{if(state.stroke?.canvas===canvas)traceStroke(event,canvas,item);};
  canvas.onpointerup=event=>{
    if(state.stroke?.canvas!==canvas||state.stroke.pointerId!==event.pointerId)return;
    traceStroke(event,canvas,item);
    const stroke=state.stroke;state.stroke=null;
    const extra={item_index:index,pixels:[...stroke.pixels.values()],...(stroke.action==="mark"?{marked:stroke.marked}:{})};
    void act(stroke.action,extra,stroke.view);
  };
  canvas.onpointercancel=event=>{
    if(state.stroke?.canvas!==canvas||state.stroke.pointerId!==event.pointerId)return;
    state.stroke=null;draw(canvas,item,art,index);controls();
  };
}
async function display(view,force=false){
  if(state.busy||state.stroke)return;
  if(!force&&state.view?.presentation_id===view?.presentation_id&&state.view?.state_id===view?.state_id){const changed=state.view?.dirty!==view?.dirty;state.view=view;controls();if(changed&&view)status(view.dirty?"Click Save when you are ready.":"Saved.");return;}
  const token=++state.refresh;
  if(!view){stopAnimations();state.view=null;$("pictures").replaceChildren();$("title").textContent="Waiting for artwork.";$("note").textContent="Tell your agent what you want to see.";$("note").hidden=false;$("hint").hidden=true;controls();return;}
  const assets=await Promise.all(view.presentation.items.map((item,i)=>item.playback?loadAnimation(item):item.kind==="art"?loadArt(view.state.art_ids[i]):loadSource(item.source_hash)));
  if(token!==state.refresh||state.busy||state.stroke)return;
  // A completed stroke replaces the drawing nodes; keep the viewport within this same presentation.
  const samePresentation=state.view?.presentation_id===view.presentation_id;
  const scrollPositions=samePresentation?[...document.querySelectorAll(".drawing")].map(well=>({left:well.scrollLeft,top:well.scrollTop})):[];
  const focusedChoice=samePresentation&&document.activeElement?.classList.contains("candidate-choice")?document.activeElement.dataset.item:null;
  stopAnimations();if(state.view?.presentation_id!==view.presentation_id){state.colors.clear();state.paletteChoices.clear();state.marking=!view.presentation.candidate_choices&&view.presentation.items.some((item,index)=>markable(item)&&assets[index].target.allowed_indices.length>MAX_VISIBLE_COLORS);}state.view=view;
  $("title").textContent=view.presentation.title;$("note").textContent=view.presentation.note;$("note").hidden=!view.presentation.note;$("hint").hidden=!editable();$("pictures").replaceChildren();
  view.presentation.items.forEach((item,index)=>{
    const figure=document.createElement("figure");figure.className="picture";figure.dataset.item=String(index);const caption=document.createElement("figcaption");caption.textContent=item.label;const well=document.createElement("div");well.className="drawing";const canvas=document.createElement("canvas");canvas.width=item.region.width*item.scale;canvas.height=item.region.height*item.scale;canvas.setAttribute("aria-label",item.label);well.append(canvas);figure.append(caption,well);
    if(item.playback){animate(canvas,assets[index]);}
    else if(item.kind==="reference"){const ctx=canvas.getContext("2d"),r=item.region;ctx.imageSmoothingEnabled=false;checker(ctx,r.width,r.height,item.scale);ctx.drawImage(assets[index],r.x,r.y,r.width,r.height,0,0,canvas.width,canvas.height);}
    else {const art=assets[index];canvas.className=item.editable?"editable markable":"markable";draw(canvas,item,art,index);attachBrush(canvas,item,index,art);if(item.editable){const palette=document.createElement("div");palette.className="palette";palette.setAttribute("role","group");palette.setAttribute("aria-label",`${item.label} colors`);
      if(!state.paletteChoices.has(index))state.paletteChoices.set(index,visibleColors(art,item.region));
      for(const color of state.paletteChoices.get(index)){const swatch=document.createElement("button");swatch.className="swatch";swatch.dataset.index=String(color);swatch.dataset.item=String(index);const transparent=color===art.target.transparent_index;if(transparent)swatch.classList.add("transparent");else swatch.style.backgroundColor=art.target.palette[color];swatch.title=transparent?"Erase":art.target.palette[color];swatch.setAttribute("aria-label",transparent?"Erase":`Color ${art.target.palette[color]} · ${color+1}`);swatch.setAttribute("aria-pressed",String(state.colors.get(index)===color));swatch.onclick=()=>{state.colors.set(index,color);state.marking=false;controls();status("Click the pixels you want to change.");};palette.append(swatch);}figure.append(palette);
      if(art.target.allowed_indices.length>MAX_VISIBLE_COLORS){const note=document.createElement("p");note.textContent="Showing up to 20 common colors. Mark areas to change and ask your agent to edit them.";figure.append(note);}
    }}
    if(choiceIndices().includes(index)){
      const button=document.createElement("button");button.className="candidate-choice";button.dataset.item=String(index);button.setAttribute("aria-label",`Choose ${item.label}`);
      button.onclick=()=>{const selected=chosenIndices();void act("choose",{item_indices:selected.includes(index)?selected.filter(i=>i!==index):[...selected,index]});};figure.append(button);
    }
    $("pictures").append(figure);
  });
  document.querySelectorAll(".drawing").forEach((well,index)=>{const position=scrollPositions[index];if(position){well.scrollLeft=position.left;well.scrollTop=position.top;}});
  if(focusedChoice!==null)document.querySelector(`.candidate-choice[data-item="${focusedChoice}"]`)?.focus({preventScroll:true});
  controls();status((editable()||canMark()||choiceIndices().length)?(view.dirty?"Click Save when you are ready.":"Saved."):"");
}
function actionMessage(pending){
  const action=pending.input.action,error=pending.error;
  if(pending.completed){
    if(state.view?.presentation_id!==pending.input.presentation_id)status("Your earlier change was recovered. Please check the current view.");
    return;
  }
  status(error?.code==="presentation_conflict"?"New artwork has arrived. Please check the current view.":error?.code==="protected_pixel"||error?.code==="out_of_bounds"?"This area is protected. Tell your agent where you want to edit.":action==="mark"&&error?.code==="limit_exceeded"?"Too many marked pixels. Unmark some pixels or use a smaller area.":action==="save"?"Save was not applied. Your previous saved work is preserved.":"Change was not applied. Please try again.",true);
}
async function synchronize(force=false){
  if(state.connection==="closed"||state.polling||state.busy||state.stroke)return;
  const flight={};state.polling=flight;let token=state.refresh;
  try{
    if(state.connection!=="ready"){
      const instance=await json("/api/workbench");
      if(token!==state.refresh)return;
      if(state.workbenchId&&state.workbenchId!==instance.workbench_id){const error=new Error("The workbench has changed.");error.code="workbench_conflict";throw error;}
      state.workbenchId=instance.workbench_id;state.activityInterval=Math.min(30000,instance.idle_timeout_seconds*250);
    }
    const pending=state.pending;
    if(pending&&pending.completed===undefined){
      const receipt=await json("/api/presentation/recover",{method:"POST",headers:{"Content-Type":"application/json","x-dotmend-action":pending.id},body:JSON.stringify(pending.input)});
      if(token!==state.refresh)return;
      pending.completed=receipt.completed;
    }
    // Keep action completion across retries, but always read the current view.
    const data=await json("/api/presentation");
    if(token!==state.refresh)return;
    const displaying=display(data.presentation,force||!!pending||state.connection!=="ready");token=state.refresh;await displaying;
    if(token!==state.refresh)return;
    state.pending=null;state.connection="ready";controls();
    if(pending){actionMessage(pending);if(pending.focusedChoice!==null&&state.view?.presentation_id===pending.input.presentation_id)document.querySelector(`.candidate-choice[data-item="${pending.focusedChoice}"]`)?.focus({preventScroll:true});}
  }catch(error){if(token===state.refresh&&!state.busy&&!state.stroke)connectionFailed(error);}
  finally{if(state.polling===flight)state.polling=null;}
}
async function act(action,extra={},view=state.view){
  if(!view||!canAct())return;
  const pending={id:crypto.randomUUID(),input:{action,presentation_id:view.presentation_id,expected_state_id:view.state_id,...extra},focusedChoice:document.activeElement?.classList.contains("candidate-choice")?document.activeElement.dataset.item:null};
  state.pending=pending;state.busy=true;state.polling=null;state.refresh++;controls();status(action==="save"?"Saving…":"Applying your change…");
  try{await json("/api/presentation/action",{method:"POST",headers:{"Content-Type":"application/json","x-dotmend-action":pending.id},body:JSON.stringify(pending.input)});pending.completed=true;}
  catch(error){pending.error=error;}
  finally{state.busy=false;}
  state.connection="checking";
  // Restore the confirmed view while an uncertain action is settled by the server.
  try{await display(state.view,true);}catch(error){connectionFailed(error);}
  if(pending.error?.code==="workbench_conflict"){connectionFailed(pending.error);return;}
  status(pending.completed===undefined?"Checking whether your change was applied…":"Checking the current view…");
  await synchronize(true);
}

$("mark").onclick=()=>{state.marking=!state.marking;controls();status(state.marking?"Marked pixels are notes for your agent. The artwork stays unchanged.":choiceIndices().length?choiceHint():editable()?"Choose a palette color to paint.":"Use Mark issues to point out pixels for your agent.");};
$("undo").onclick=()=>void act("undo");$("save").onclick=()=>void act("save");
document.addEventListener("keydown",event=>{if(!(event.metaKey||event.ctrlKey)||event.altKey)return;const key=event.key.toLowerCase();if(key!=="z"&&key!=="s")return;event.preventDefault();if(event.shiftKey)return;const button=$(key==="z"?"undo":"save");if(!button.disabled)button.click();});
// Only real input renews idle time; polling, playback and unattended tabs do not.
for(const type of ["pointerdown","pointermove","pointerup","wheel","keydown"])document.addEventListener(type,renewActivity,{capture:true,passive:true});
window.addEventListener("focus",()=>void synchronize());window.addEventListener("online",()=>void synchronize());setInterval(()=>{if(document.visibilityState==="visible")void synchronize();},1000);controls();void synchronize();
