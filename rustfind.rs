#[feature(managed_boxes)];
#[feature(globs)];
#[feature(macro_rules)];
#[feature(log_syntax)];

extern crate syntax;
extern crate rustc;
extern crate getopts;
extern crate serialize;
extern crate collections;

use std::io::println;

use rustc::{driver};
use rustc::metadata::creader::Loader;

use std::{os,local_data};
use std::cell::RefCell;
use collections::{HashMap,HashSet};
use std::path::Path;

use getopts::{optmulti, optopt, optflag, getopts};

use syntax::{parse,ast,codemap};
use syntax::codemap::Pos;
use find_ast_node::{safe_node_id_to_type,get_node_info_str,find_node_tree_loc_at_byte_pos,ToJsonStr,ToJsonStrFc,AstNodeAccessors,get_node_source};
use jumptodefmap::{lookup_def_at_text_file_pos_str, make_jdm, def_info_from_node_id,
    lookup_def_at_text_file_pos, dump_json};

use rfindctx::{RFindCtx,ctxtkey};
pub use codemaput::{ZTextFilePos,ZTextFilePosLen,get_span_str,ToZTextFilePos,ZIndexFilePos,ToZIndexFilePos};
use rsfind::{SDM_LineCol,SDM_Source,SDM_GeditCmd};
use crosscratemap::CrossCrateMap;

pub mod rf_common;
pub mod find_ast_node;
pub mod text_formatting;
pub mod ioutil;
pub mod htmlwriter;
pub mod rust2html;
pub mod codemaput;
pub mod rfindctx;
pub mod rsfind;
pub mod crosscratemap;
pub mod rfserver;
pub mod util;
pub mod rf_ast_ut;
pub mod jumptodefmap;
pub mod timer;

pub macro_rules! tlogi{
    ($($a:expr),*)=>(println!((file!()+":"+line!().to_str()+": " $(+$a.to_str())*) ))
}
pub macro_rules! logi{
    ($($a:expr),*)=>(println(""$(+$a.to_str())*) )
}
macro_rules! dump{ ($($a:expr),*)=>
    (   {   let mut txt=~"";
            $( { txt=txt.append(
                 format!("{:s}={:?}",stringify!($a),$a)+",")
                }
            );*;
            logi!(txt);
        }
    )
}

pub macro_rules! if_some {
    ($b:ident in $a:expr then $c:expr)=>(
        match $a {
            Some($b)=>$c,
            None=>{}
        }
    );
    ($b:ident in $a:expr then $c:expr _else $d:expr)=>(
        match $a {
            Some($b)=>$c,
            None=>{$d}
        }
    );
}

fn optgroups () -> ~[getopts::OptGroup] {
    ~[
        optmulti("L", "", "Path to search for libraries", "<lib path>"),
        optflag("d", "", "Debug for this tool (DEBUG)"),
        optflag("r", "", "Dump .rfx `crosscratemap` containing ast nodes and jump definitions"),
        optflag("j", "", "Dump json map of the ast nodes & definitions (DEBUG)"),
        optflag("h", "help", "Print this help menu and exit"),
        optflag("i", "", "Run interactive server"),
        optflag("g", "", "Format output as gedit `filepos +line filename` (use with -f)"),
        optflag("w", "", "Dump as html"),
        optflag("f", "", "Return definition reference of symbol at given position"),
        optopt("x", "", "Path to html of external crates", "RUST_SRC"),
        optopt("o", "", "Directory to output the html / rfx files to", "OUTDIR")
    ]
}

fn usage(binary: &str) {
    let message = format!("Usage: {} [OPTIONS] INPUT", binary);
    println!("{}", getopts::usage(message, optgroups()));

    println!(" cratename.rs anotherfile.rs:line:col:");
    println!("    - load cratename.rs; look for definition at anotherfile.rs:line:col");
    println!("    - where anotherfile.rs is assumed to be a module of the crate");
    println!(" Set RUST_LIBS for a default library search path");
}

fn main() {
    let mut args = os::args();
    let binary = args.shift().unwrap();

    let opts = optgroups();
	let matches = getopts(args, opts).unwrap();
    let libs1 = matches.opt_strs("L").map(|s| Path::new(s.as_slice()));
    let libs= if libs1.len() > 0 {
        libs1
    } else {
        match os::getenv(&"RUST_LIBS") {
            Some(x) => ~[Path::new(x)],
            None => ~[]
        }
    };

    if matches.opt_present("h") || matches.opt_present("help") {
        usage(binary);
        return;
    };
    if matches.free.len()>0 {
        let mut done=false;
        let filename=util::get_filename_only(matches.free[0]);
        let dc = @get_ast_and_resolve(&Path::new(filename), libs.move_iter().collect());
        local_data::set(ctxtkey, dc);

        if matches.opt_present("d") {
            debug_test(dc);
            done=true;
        } else if matches.opt_present("j"){
            dump_json(dc);
        }
        let mut i=0;
        let lib_html_path = Path::new(if matches.opt_present("x") {
            matches.opt_str("x").unwrap()
        } else {
            ~""
        });
        if matches.opt_present("f") {
            while i<matches.free.len() {
                let mode=if matches.opt_present("g"){SDM_GeditCmd} else {SDM_Source};
                println!("{}", lookup_def_at_text_file_pos_str(dc,matches.free[i],mode).unwrap_or(~"no def found"));
                i+=1;
                done=true;
            }
        }
        if matches.opt_present("i") {
            rfserver::run_server(dc);
            done=true;
        }
        let mut html_options = rust2html::Options::default();
        if matches.opt_present("o") {
            let out_dir = matches.opt_str("o").unwrap();
            let mut out_path = Path::new("./");
            out_path.push(out_dir + "/");
            html_options.output_dir = out_path;
        }
        if matches.opt_present("r") {
            println!("Writing .rfx ast nodes/cross-crate-map:-");
            write_source_as_html_and_rfx(dc, &lib_html_path, &html_options, false);
            println!("Writing .rfx .. done");
            done=true;
        }

        // Dump as html..
        if matches.opt_present("w") || !(done) {
            println!("Creating HTML pages from source & .rfx:-");
            write_source_as_html_and_rfx(dc, &lib_html_path, &html_options, true);
            println!("Creating HTML pages from source.. done");
        }
	} else {
        usage(binary);
        return;
    }
}

/// tags: crate,ast,parse resolve
/// Parses, resolves the given crate

fn get_ast_and_resolve(
    cpath: &std::path::Path,
    libs: HashSet<std::path::Path>)
    -> RFindCtx {

    let parsesess = parse::new_parse_sess();
    let sessopts = @driver::session::Options {
        maybe_sysroot: Some(@std::os::self_exe_path().unwrap()),
        addl_lib_search_paths:  @RefCell::new(libs),
        ..  (*rustc::driver::session::basic_options()).clone()
    };
    // Currently unused
//  let quiet = true;


    let diagnostic_handler = syntax::diagnostic::default_handler(); // TODO add back the blank emitter here ...
    let span_diagnostic_handler =
        syntax::diagnostic::mk_span_handler(diagnostic_handler, parsesess.cm);


    let sess = driver::driver::build_session_(sessopts, 
                                              Some(cpath.clone()), 
                                              parsesess.cm,
                                              span_diagnostic_handler);
    let input=driver::driver::FileInput(cpath.clone());
    let cfg = driver::driver::build_configuration(sess); //was, @"", &input);

    let crate1 = driver::driver::phase_1_parse_input(sess,cfg.clone(),&input);
    let loader = &mut Loader::new(sess);
    let (crate2, ast_map) = driver::driver::phase_2_configure_and_expand(sess, loader, crate1);

    let ca = driver::driver::phase_3_run_analysis_passes(sess, &crate2, ast_map);
    RFindCtx { crate_: @crate2, tycx: ca.ty_cx, sess: sess, ca:ca }
}

fn debug_test(dc:&RFindCtx) {

    // TODO: parse commandline source locations,convert to codemap locations
    //dump!(ctxt.tycx);
    logi!("==== Get tables of node-spans,def_maps,JumpToDefTable..===")
    let (node_info_map,node_def_node,jdm)=jumptodefmap::make_jdm(dc);
    println(node_info_map.to_json_str(dc));

    logi!("==== Node Definition mappings...===")
    println(node_def_node.to_json_str());

    logi!("==== JumpToDef Table ===");
    println(jdm.to_json_str());


    logi!("==== dump def table.===");
    rf_ast_ut::dump_ctxt_def_map(dc.tycx);

    logi!("==== Test node search by location...===");

    // Step a test 'cursor' src_pos through the given source file..
    let mut test_cursor=15 as uint;

    while test_cursor<500 {
        let loc = rfindctx::get_source_loc(dc,codemap::BytePos(test_cursor as u32));

        logi!(~"\n=====Find AST node at: ",loc.file.name,":",loc.line,":",loc.col.to_uint().to_str(),":"," =========");

        let nodetloc = find_node_tree_loc_at_byte_pos(dc.crate_,codemap::BytePos(test_cursor as u32));
        let node_info =  get_node_info_str(dc,&nodetloc);
        dump!(node_info);
        println("node ast loc:"+(nodetloc.map(|x| { x.get_id().to_str() })).to_str());


        if_some!(id in nodetloc.last().get_ref().ty_node_id() then {
            logi!("source=",get_node_source(dc.tycx, &node_info_map,ast::DefId{krate:0,node:id}));
            if_some!(t in safe_node_id_to_type(dc.tycx, id) then {
                println!("typeinfo: {:?}",
                    {let ntt= rustc::middle::ty::get(t); ntt});
                dump!(id,dc.tycx.def_map.borrow().get().find(&id));
                });
            let (def_id,opt_info)= def_info_from_node_id(dc,&node_info_map,id);
            if_some!(info in opt_info then{
                logi!("src node=",id," def node=",def_id,
                    " span=",format!("{:?}",info.span));
                logi!("def source=", get_node_source(dc.tycx, &node_info_map, def_id));
            })
        })

        test_cursor+=20;
    }

    // test byte pos from file...
    logi!("====test file:pos source lookup====");
    dump!(ZTextFilePosLen::new("test_input.rs",3-1,0,10).get_str(dc.tycx));
    dump!(ZTextFilePosLen::new("test_input.rs",9-1,0,10).get_str(dc.tycx));
    dump!(ZTextFilePosLen::new("test_input2.rs",4-1,0,10).get_str(dc.tycx));
    dump!(ZTextFilePosLen::new("test_input2.rs",11-1,0,10).get_str(dc.tycx));
    let ospan=ZTextFilePosLen::new("test_input2.rs", 10-1,0,32).to_byte_pos(dc.tycx);
    if_some!(x in ospan then {
        let (lo,hi)=x;
        logi!(get_span_str(dc.tycx, &codemap::Span{lo:lo,hi:hi,expn_info:None} ));
    });
    dump!(codemaput::zget_file_line_str(dc.tycx,"test_input2.rs",5-1));
    dump!(codemaput::zget_file_line_str(dc.tycx,"test_input.rs",9-1));

    logi!("\n====test full file:pos lookup====");
    dump!(lookup_def_at_text_file_pos(dc, &ZTextFilePos::new("test_input.rs",8-1,21),SDM_Source));println!("");
    dump!(lookup_def_at_text_file_pos(dc, &ZTextFilePos::new("test_input2.rs",3-1,12),SDM_Source));println!("");
    dump!(lookup_def_at_text_file_pos(dc, &ZTextFilePos::new("test_input.rs",10-1,8),SDM_Source));println!("");
    dump!(lookup_def_at_text_file_pos(dc, &ZTextFilePos::new("test_input.rs",13-1,16),SDM_Source));println!("");
    dump!(lookup_def_at_text_file_pos(dc, &ZTextFilePos::new("test_input.rs",11-1,10),SDM_Source));println!("");

}



pub fn write_source_as_html_and_rfx(dc:&RFindCtx, lib_html_path:&Path, opts: &rust2html::Options, write_html: bool) {

    let mut xcm: ~CrossCrateMap = ~HashMap::new();
    dc.tycx.cstore.iter_crate_data(|i,md| {
        //dump!(i, md.name,md.data.len(),md.cnum);
        println!("loading cross crate data {} {}", i, md.name);
        let xcm_sub=crosscratemap::read_cross_crate_map(dc, i as int, 
                                                        "lib" + md.name + "/lib.rfx", 
                                                        lib_html_path);
        for (k,v) in xcm_sub.iter() {
            xcm.insert(*k, (*v).clone());
        }
    });

    let (info_map, def_map, jump_map) = make_jdm(dc);
    crosscratemap::write_cross_crate_map(dc, &opts.output_dir, &info_map, def_map, jump_map);
    if write_html {
        rust2html::write_source_as_html_sub(dc, &info_map, jump_map, xcm, lib_html_path, opts);
    }
}


