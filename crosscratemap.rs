use rf_common::*;
use syntax::ast;
use syntax::codemap::Pos;
use find_ast_node::FNodeInfoMap;
use jumptodefmap::{JumpToDefMap};
use codemaput::ToZTextFilePos;
use ioutil;
use rfindctx::{RFindCtx, str_of_opt_ident};

/*new file*/

pub type ZeroBasedIndex=uint;

/// cross crate map, extra info written out when compiling links of a crate
/// allows sunsequent crates to jump to definitions in that crate
/// TODO - check if this already exists in the 'cstore/create metadata'
/// specificially we need node->span info
#[deriving(Clone)]
pub struct CrossCrateMapItem {
    fname:~str,
    line:ZeroBasedIndex,
    col:uint,
    len:uint
}

pub type CrossCrateMap = HashMap<ast::DefId,CrossCrateMapItem>;


pub fn read_cross_crate_map(_:&RFindCtx, crate_num:int, crate_name:&str, lib_path: &Path)->~CrossCrateMap {
    println!("loading lib crosscratemap {}/{}", lib_path.display(), crate_name);
    fn load_cross_crate_map(path: &Path) -> ~str {
        match ioutil::fileLoad(path, true) {
            Some(s) => s,
            None => ~"" // give up :(
        }
    }

    let rfx = match ioutil::fileLoad(&Path::new(crate_name), false) {
        None => load_cross_crate_map(&lib_path.join(crate_name)),
        Some(ref s) if s.len() == 0 => load_cross_crate_map(&lib_path.join(crate_name)),
        Some(s) => s
    };
    println!("loaded cratemap {} bytes as crate {}", rfx.len(), crate_num);
//  for &x in raw_bytes.iter() { rfx.push_char(x as char); }

    let mut xcm=~HashMap::new();
    for s in rfx.lines() {
//      println(s.to_str());
        let toks=s.split('\t').to_owned_vec();
        if toks.len()>=6 {
            match toks[0] {
                "jdef"=> {
                    // jimp- to def info, we dont need this here as we already generated it
                    // for the current crate. TODO , genarlized rfx would use it..
                }
                "node"=> {
                    // node cratename nodeid parentid sourcefile line col len type [ident]
                    //cratename is ignoredd, because we already know it.
                    // parent id ignored, we use span information to reconstruct AST

                    let node_id: int= from_str::<int>(toks[2]).unwrap_or(0);
                    xcm.insert(ast::DefId{krate:crate_num as u32, node:node_id as u32,},
                        CrossCrateMapItem{
                            fname:  toks[4].to_owned(),
                            line:   from_str(toks[5]).unwrap_or(0)-1,
                            col:    from_str(toks[6]).unwrap_or(0),
                            len:    from_str(toks[7]).unwrap_or(0)
                        }
                    );
                }
                // legacy noode definitons,no keyword
                _=>{

                    let node_id:int=from_str(toks[1]).unwrap_or(0);
                    xcm.insert(ast::DefId{krate:crate_num as u32, node:node_id as u32,},
                        CrossCrateMapItem{
                            fname:  toks[2].to_owned(),
                            line:   from_str(toks[3]).unwrap_or(0)-1,
                            col:    from_str(toks[4]).unwrap_or(0),
                            len:    from_str(toks[5]).unwrap_or(0)
                        }
                    );
                }
            }
        }
    }
    //dump!(xcm);
    xcm
}



pub fn write_cross_crate_map(dc: &RFindCtx, out_path: &Path, nim: &FNodeInfoMap, 
                             _: &HashMap<ast::NodeId, ast::DefId>, jdm: &JumpToDefMap) {
    // write inter-crate node map
    let crate_rel_path_name = dc.sess.codemap.files.borrow();
    let crate_rel_path_name = Path::new(crate_rel_path_name.get().get(0).name.as_slice());
    let new_format:bool=true;

    let curr_crate_name_only = crate_rel_path_name.filestem_str().unwrap_or("");
    let out_file = out_path.join(curr_crate_name_only).with_extension("rfx");

    println!("Writing rustfind cross-crate link info for {} ({})", curr_crate_name_only, out_file.display());
    let out = ioutil::file_create_with_dirs(&out_file).map(|out| {
        let mut out_file = out;
        // todo - idents to a seperate block, they're rare.
        for (k,ni) in nim.iter() {
            match ni.span.lo.to_text_file_pos(dc.tycx) {
                Some(tfp)=>{
                    // new format, a little more verbose,
                    // and includes parent id for easier reconstruction of full AST
                    // "node" cratename id parent_id filename line col len type [ident]
                    if new_format {
                        try!(writeln!(&mut out_file, "node\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                            curr_crate_name_only, k, ni.parent_id, tfp.name, (tfp.line + 1), tfp.col,
                            (ni.span.hi - ni.span.lo).to_uint(), ni.kind, str_of_opt_ident(ni.ident)));
                    } else  {
                        // old format, relies on spans to reconstruct AST.
                        // cratename id filename line col len type [ident]
                        try!(writeln!(&mut out_file, "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                            curr_crate_name_only, k, tfp.name, (tfp.line+1), tfp.col,
                            (ni.span.hi-ni.span.lo).to_uint(), ni.kind, str_of_opt_ident(ni.ident)));
                    }
                },
                None=>{}
            }
        }

        for (k,v) in jdm.iter()  {
            let cname: ~str = if v.krate > 0 {
                dc.tycx.cstore.get_crate_data(v.krate).name.to_str()
            } else {
                curr_crate_name_only.to_str()
            };
            //println(cdata.name);
            try!(writeln!(&mut out_file, "jdef\t{}\t{}\t{}", k, cname, v.node));
        }

        Ok(())
    });

    match out {
        Err(e) => println!("Error while writing to {}: {}", out_path.display(), e),
        _ => ()
    };

//  for (k,v) in ndm.iter()  {
//      outp.push_str("def\t"+k.to_str()+"\t"+dc.tycx.cstore.crate() +v.node.to_str()+"\n");
//  }
}
