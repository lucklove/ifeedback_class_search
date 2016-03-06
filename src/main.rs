extern crate hyper;
extern crate rustc_serialize;
extern crate mysql;

use std::io::Read;
use std::collections::HashMap;
use hyper::Client;
use hyper::method::Method;
use hyper::server::{Handler, Server, Request, Response};
use rustc_serialize::json;
use mysql as my;
use hyper::uri::RequestUri::AbsolutePath;
use std::sync::Mutex;

#[cfg(test)]
mod tests {
    #[test]
    fn test_string_tokenizer() {
        let v = super::string_tokenizer("南京 市长  江大桥");
        assert_eq!(v, vec!["南京", "市长", "江大桥"]);    
    }

    #[test]
    fn test_cut() {
        let s = "工信处女干事每月经过下属科室都要亲口交代24口交换机等技术性器件的安装工作";
        let words = super::cut_for_search(s).unwrap();
        assert_eq!(words, vec!["工信处", "女干事", "每月", "经过", "下属", "科室", "都", 
            "要", "亲口", "交代", "24", "口", "交换机", "等", "技术性", "器件", "的", "安装", "工作"])
    }

    #[test]
    fn test_num_to_zh() {
        assert_eq!(super::num_to_zh("0").unwrap(), "零");
        assert_eq!(super::num_to_zh("12345").unwrap(), "一万二千三百四十五");
        assert_eq!(super::num_to_zh("1001").unwrap(), "一千零一");
        assert_eq!(super::num_to_zh("19").unwrap(), "十九");
        assert_eq!(super::num_to_zh("119").unwrap(), "一百一十九");
    }
}

fn string_tokenizer(s : &str) -> Vec<String> {
    let mut v : Vec<String> = Vec::new();
    let mut string = String::new();
    for c in s.chars() {
        if c.is_whitespace() {
            if !string.is_empty() {
                v.push(string.clone());
                string.clear();
            }
            continue; 
        } else {
            string.push(c)
        }
    }
    if !string.is_empty() {
        v.push(string);
    }
    v
}

fn cut_for_search(s : &str) -> Result<Vec<String>, hyper::error::Error> {
    let client = Client::new();
    let url = "http://127.0.0.1:1025/?key=".to_string() + s + "&format=simple&method=QUERY";
    let mut res = try!(client.get(&url).send());
    let mut body = String::new();
    try!(res.read_to_string(&mut body));
    Ok(string_tokenizer(&body))
}
 
//万级以下的数字转中文
fn num_to_zh(num_str : &str) -> Option<String> {
    let digit_chinese_name = ['零', '一', '二', '三', '四', '五', '六', '七', '八', '九'];
    let mut s = String::new();
    if let Ok(mut num) = num_str.parse::<usize>() {
        loop {
            match num {
                0 => { 
                    if s.is_empty() { 
                        s.push('零'); 
                    } 
                    return Some(s); 
                },
                n @ 1 ... 9 => { 
                    s.push(digit_chinese_name[n]); 
                    return Some(s); 
                },
                n @ 10 ... 99 => {
                    if n > 20 || !s.is_empty() {
                        s.push(digit_chinese_name[n/10]);
                    }
                    s.push('十');
                    num %= 10;
                },
                n @ 100 ... 999 => {
                    s.push(digit_chinese_name[n/100]);
                    s.push('百');
                    num %= 100;
                    if num < 10 && num > 0 {
                        s.push('零');
                    }
                },
                n @ 1000 ... 9999 => {
                    s.push(digit_chinese_name[n/1000]);
                    s.push('千');
                    num %= 1000;
                    if num < 100 && num > 0 {
                        s.push('零');
                    }
                },
                n @ 10000 ... 99999 => {
                    s.push(digit_chinese_name[n/10000]);
                    s.push('万');
                    num %= 10000;
                    if num < 1000 && num > 0 {
                        s.push('零');
                    }
                },
                _ => {}
            }
        }
    } else {
        None    
    } 
}

#[derive(RustcDecodable)]
struct RequestObject {
    content: String
}

#[derive(RustcEncodable)]
struct ResponseObject {
    result: Vec<u32>
}

struct ClassSearcher {
    //关键词，和该关键词相关的所有班级id
    keyword_map: Mutex<HashMap<String, Vec<u32>>>,
}

impl ClassSearcher {
    fn new() -> ClassSearcher {
        let searcher = ClassSearcher {
            keyword_map: Mutex::new(HashMap::<String, Vec<u32>>::new())
        };
        searcher.reload_keyword_map();
        searcher
    }

    fn reload_keyword_map(&self) {
        let pool = my::Pool::new("mysql://root:lucklove@127.0.0.1").unwrap();
        self.keyword_map.lock().unwrap().clear();
        let _ : Vec<()> = pool.prep_exec("select id, keyword from db_class_search.tb_keyword", ()).map(|result| {
            result.map(|x| x.unwrap()).map(|row| {
                let (id, key) = my::from_row::<(u32, String)>(row);
                let mut keyword_map = self.keyword_map.lock().unwrap();
                if !keyword_map.contains_key(&key) {
                    keyword_map.insert(key.clone(), vec![id]);
                } else {
                    keyword_map.get_mut(&key).unwrap().push(id);
                }
            }).collect()
        }).unwrap();
    }

    fn handle_search(&self, req: &mut Request, mut res: Response) {
        let mut s = String::new();
        match req.read_to_string(&mut s) {
            Ok(_) => { 
                match json::decode::<RequestObject>(&s) {
                    Ok(r) => {
                        if let Ok(mut keys) = cut_for_search(&r.content) {
                            let mut adjust_keys = Vec::<String>::new();
                            for key in &keys {
                                if let Some(k) = num_to_zh(key) {
                                    adjust_keys.push(k);
                                }
                            }
                            keys.extend_from_slice(&adjust_keys);
                            let mut matched_map : HashMap<u32, usize> = HashMap::new();
                            for key in &keys {
                                if let Some(v) = self.keyword_map.lock().unwrap().get(key) {
                                    for id in v {
                                        if !matched_map.contains_key(id) {
                                            matched_map.insert(*id, 1);
                                        } else {
                                            *matched_map.get_mut(id).unwrap() += 1;
                                        }
                                    }
                                }
                            }
                            let mut matched : Vec<(u32, usize)> = Vec::new();
                            for (id, count) in matched_map {
                                matched.push((id, count));
                            }
                            matched.sort_by(|a: &(u32, usize), b: &(u32, usize)| {
                                let (_, ref l) = a.clone();
                                let (_, ref r) = b.clone();
                                r.cmp(l)
                            });
                            let mut result : Vec<u32> = Vec::new();
                            for (id, _) in matched {
                                result.push(id);
                            }
                            let output = ResponseObject { result: result };
                            let _ = res.send(json::encode(&output).unwrap().as_bytes()); 
                        } else { 
                            *res.status_mut() = hyper::BadRequest;
                        }
                    },
                    _ => { *res.status_mut() = hyper::BadRequest; }
                }
            },
            _ => { *res.status_mut() = hyper::BadRequest; }
        }
    }

    fn handle_set(&self, req: &mut Request, mut res: Response) {
        if let AbsolutePath(path) = req.uri.clone() {
            if path.len() < 5 {
                *res.status_mut() = hyper::BadRequest;
                return;
            }
            if let Ok(id) = path[5..].parse::<u32>() {
                let mut s = String::new();
                if let Ok(_) = req.read_to_string(&mut s) {
                    if let Ok(r) = json::decode::<RequestObject>(&s) {
                        let keys = cut_for_search(&r.content).unwrap();
                        let pool = my::Pool::new("mysql://root:lucklove@127.0.0.1").unwrap();
                        pool.prep_exec(r"delete from db_class_search.tb_keyword where id=?", (id,)).unwrap();
                        let mut keyword_map = self.keyword_map.lock().unwrap();
                        println!("pre");
                        for mut stmt in pool.prepare(r"insert into db_class_search.tb_keyword(id, keyword) values(?, ?)")
                            .into_iter() {
                            println!("pre in");
                            for k in &keys {
                                println!("in");
                                stmt.execute((id, k)).unwrap();
                                if !keyword_map.contains_key(k) {
                                    keyword_map.insert(k.clone(), vec![id]);
                                } else {
                                    keyword_map.get_mut(k).unwrap().push(id);
                                }
                            }   
                        }
                        let _ = res.send("{\"result\":\"ok\"}".as_bytes());
                    } else {
                        *res.status_mut() = hyper::BadRequest;
                        return;
                    }
                } else {
                    *res.status_mut() = hyper::BadRequest;
                    return;
                }
            } else {
                *res.status_mut() = hyper::BadRequest;
                return;
            }
        } else {
            panic!("path error");
        }
    }

    fn handle_del(&self, req: &mut Request, mut res: Response) {
        if let AbsolutePath(ref path) = req.uri {
            if path.len() < 5 {
                *res.status_mut() = hyper::BadRequest;
                return;
            }
            if let Ok(id) = path[5..].parse::<usize>() {
                let pool = my::Pool::new("mysql://root:lucklove@127.0.0.1").unwrap();
                pool.prep_exec(r"delete from db_class_search.tb_keyword where id=?", (id,)).unwrap();
                let _ = res.send("{\"result\":\"ok\"}".as_bytes());
            } else {
                *res.status_mut() = hyper::BadRequest;
                return;
            }
        } else {
            panic!("path error");
        }
    }
}

impl Handler for ClassSearcher {
    fn handle(&self, mut req: Request, mut res: Response) {
        match req.uri.clone() {
            AbsolutePath(path) => match (req.method.clone(), &path[0..4]) {
                (Method::Post, "/sea") => {
                    self.handle_search(&mut req, res);
                },
                (Method::Post, "/set") => {
                    self.handle_set(&mut req, res);
                },
                (Method::Delete, "/del") => {
                    self.handle_del(&mut req, res);
                },
                _ => *res.status_mut() = hyper::NotFound
            },
            _ => {}
        }
    }
}

fn main() {
    Server::http("0.0.0.0:1024").unwrap().handle(ClassSearcher::new()).unwrap(); 
}
