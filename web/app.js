"use strict";
const $=id=>document.getElementById(id);
const state={workbenchId:null,view:null,arts:new Map(),sources:new Map(),colors:new Map(),marking:false,stroke:null,busy:false,refresh:0,polling:false,timers:new Set()};
function status(text,error=false){$("status").textContent=text;$("status").classList.toggle("error",error);}
async function json(url,options){const response=await fetch(url,{cache:"no-store",...options,headers:{...options?.headers,...(state.workbenchId?{"x-retro-art-workbench":state.workbenchId}:{})}});const data=await response.json();if(!response.ok||data.ok===false){const error=new Error(data.error?.message||"Please check your connection.");error.code=data.error?.code;throw error;}return data;}
function loadArt(id){if(!state.arts.has(id))state.arts.set(id,json(`/api/art/${id}`).catch(error=>{state.arts.delete(id);throw error;}));return state.arts.get(id);}
function loadSource(hash){if(!state.sources.has(hash))state.sources.set(hash,new Promise((resolve,reject)=>{const img=new Image();img.onload=()=>resolve(img);img.onerror=()=>{state.sources.delete(hash);reject(new Error("Could not load the reference image."));};img.src=`/api/source/${hash}`;}));return state.sources.get(hash);}
async function loadAnimation(item){const result=await json("/api/call",{method:"POST",headers:{"Content-Type":"application/json"},body:JSON.stringify({tool:"render_art_set",arguments:{frames:item.playback,scale:item.scale,view:"frames"}})});result.loaded=await Promise.all(result.images.map(src=>new Promise((resolve,reject)=>{const image=new Image();image.onload=()=>resolve(image);image.onerror=()=>reject(new Error("Could not load the frames."));image.src=src;})));return result;}
function stopAnimations(){for(const timer of state.timers)clearTimeout(timer);state.timers.clear();}
function later(fn,delay){const timer=setTimeout(()=>{state.timers.delete(timer);fn();},Math.min(delay,60000));state.timers.add(timer);}
function animate(canvas,result){let index=0;const token=state.refresh;canvas.width=result.display_width;canvas.height=result.display_height;const ctx=canvas.getContext("2d");ctx.imageSmoothingEnabled=false;function frame(){if(token!==state.refresh)return;ctx.clearRect(0,0,canvas.width,canvas.height);checker(ctx,Math.ceil(canvas.width/8),Math.ceil(canvas.height/8),8);ctx.drawImage(result.loaded[result.frames[index].image_index],0,0);const end=performance.now()+result.frames[index].duration_ms;function wait(){if(token!==state.refresh)return;const remaining=end-performance.now();if(remaining>0)later(wait,remaining);else{index=(index+1)%result.frames.length;frame();}}later(wait,result.frames[index].duration_ms);}frame();}
function editable(){return state.view?.presentation.items.some(i=>i.kind==="art"&&i.editable);}
function markable(item){return item.kind==="art"&&!item.playback;}
function canMark(){return state.view?.presentation.items.some(markable);}
function controls(){
  const working=state.busy||!!state.stroke;
  $("undo").disabled=working||!state.view?.undo_available;
  $("save").disabled=working||!(editable()||canMark())||!state.view?.dirty;
  $("mark").disabled=working||!canMark();$("mark").setAttribute("aria-pressed",String(state.marking));
  $("hint").hidden=!(editable()||canMark());
  $("hint").textContent=state.marking?"Left-click or drag to mark pixels. Right-click or drag to unmark. Choose a palette color to paint.":editable()?"Choose a color, then click the pixels you want to change. Use Mark issues to point out an area.":"Use Mark issues to point out pixels for your agent.";
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
    if(state.busy||state.stroke||!(state.marking?[0,2].includes(event.button):item.editable&&event.button===0))return;
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
  stopAnimations();if(state.view?.presentation_id!==view.presentation_id){state.colors.clear();state.marking=false;}state.view=view;
  $("title").textContent=view.presentation.title;$("note").textContent=view.presentation.note;$("note").hidden=!view.presentation.note;$("hint").hidden=!editable();$("pictures").replaceChildren();
  view.presentation.items.forEach((item,index)=>{
    const figure=document.createElement("figure");figure.className="picture";figure.dataset.item=String(index);const caption=document.createElement("figcaption");caption.textContent=item.label;const well=document.createElement("div");well.className="drawing";const canvas=document.createElement("canvas");canvas.width=item.region.width*item.scale;canvas.height=item.region.height*item.scale;canvas.setAttribute("aria-label",item.label);well.append(canvas);figure.append(caption,well);
    if(item.playback){animate(canvas,assets[index]);}
    else if(item.kind==="reference"){const ctx=canvas.getContext("2d"),r=item.region;ctx.imageSmoothingEnabled=false;checker(ctx,r.width,r.height,item.scale);ctx.drawImage(assets[index],r.x,r.y,r.width,r.height,0,0,canvas.width,canvas.height);}
    else {const art=assets[index];canvas.className=item.editable?"editable markable":"markable";draw(canvas,item,art,index);attachBrush(canvas,item,index,art);if(item.editable){const palette=document.createElement("div");palette.className="palette";palette.setAttribute("role","group");palette.setAttribute("aria-label",`${item.label} colors`);
      for(const color of art.target.allowed_indices){const swatch=document.createElement("button");swatch.className="swatch";swatch.dataset.index=String(color);swatch.dataset.item=String(index);const transparent=color===art.target.transparent_index;if(transparent)swatch.classList.add("transparent");else swatch.style.backgroundColor=art.target.palette[color];swatch.title=transparent?"Erase":art.target.palette[color];swatch.setAttribute("aria-label",transparent?"Erase":`Color ${art.target.palette[color]} · ${color+1}`);swatch.setAttribute("aria-pressed",String(state.colors.get(index)===color));swatch.onclick=()=>{state.colors.set(index,color);state.marking=false;controls();status("Click the pixels you want to change.");};palette.append(swatch);}figure.append(palette);
    }}$("pictures").append(figure);
  });controls();status((editable()||canMark())?(view.dirty?"Click Save when you are ready.":"Saved."):"");
}
async function synchronize(force=false){if(!state.workbenchId||state.polling||state.busy||state.stroke)return;state.polling=true;const token=state.refresh;try{const data=await json("/api/presentation");if(token===state.refresh)await display(data.presentation,force);}catch{if(token===state.refresh&&!state.busy&&!state.stroke)status("Connection lost. Checking your saved work.",true);}finally{state.polling=false;}}
async function act(action,extra={},view=state.view){
  if(!view||state.busy||state.stroke)return;state.busy=true;state.refresh++;controls();status(action==="save"?"Saving…":"");
  let error=null;let receipt=null;
  try{receipt=await json("/api/presentation/action",{method:"POST",headers:{"Content-Type":"application/json"},body:JSON.stringify({action,presentation_id:view.presentation_id,expected_state_id:view.state_id,...extra})});}
  catch(cause){error=cause;}
  finally{state.busy=false;}
  if(receipt)await display(receipt.presentation,true);else {await display(state.view,true);await synchronize(true);}
  controls();
  if(error)status(action==="mark"&&error.code==="limit_exceeded"?"Too many marked pixels. Unmark some pixels or use a smaller area.":error.code==="protected_pixel"||error.code==="out_of_bounds"?"This area is protected. Tell your agent where you want to edit.":(error.code==="presentation_conflict"||error.code==="workbench_conflict")?"New artwork has arrived. Please check the current view.":(action==="save"?"Save was not confirmed. Please check your saved work.":"Change was not confirmed. Please check your saved work."),true);
}
$("mark").onclick=()=>{state.marking=!state.marking;controls();status(state.marking?"Marked pixels are notes for your agent. The artwork stays unchanged.":"Choose a palette color to paint.");};
$("undo").onclick=()=>void act("undo");$("save").onclick=()=>void act("save");
document.addEventListener("keydown",event=>{if(!(event.metaKey||event.ctrlKey)||event.altKey)return;const key=event.key.toLowerCase();if(key!=="z"&&key!=="s")return;event.preventDefault();if(event.shiftKey)return;const button=$(key==="z"?"undo":"save");if(!button.disabled)button.click();});
window.addEventListener("focus",()=>void synchronize());setInterval(()=>{if(document.visibilityState==="visible")void synchronize();},1000);void json("/api/workbench").then(instance=>{state.workbenchId=instance.workbench_id;return synchronize();}).catch(()=>status("This view has closed. Ask your agent to reopen it."));
