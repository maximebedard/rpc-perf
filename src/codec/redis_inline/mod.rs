//  rpc-perf - RPC Performance Testing
//  Copyright 2015 Twitter, Inc
//
//  Licensed under the Apache License, Version 2.0 (the "License");
//  you may not use this file except in compliance with the License.
//  You may obtain a copy of the License at
//
//      http://www.apache.org/licenses/LICENSE-2.0
//
//  Unless required by applicable law or agreed to in writing, software
//  distributed under the License is distributed on an "AS IS" BASIS,
//  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//  See the License for the specific language governing permissions and
//  limitations under the License.

mod gen;
mod parse;

use cfgtypes;
use cfgtypes::{BenchmarkWorkload, CResult, Parameter, ParsedResponse, ProtocolConfig, ProtocolGen,
               ProtocolParse, ProtocolParseFactory, Ptype};
use cfgtypes::tools;
use getopts::Matches;
use std::collections::BTreeMap;
use std::str;
use std::sync::Arc;
use toml::Value;
use crypto::digest::Digest;
use crypto::sha1::Sha1;

type Param = Parameter<RedisData>;

#[derive(Clone, Debug)]
struct RedisData {
    size: usize,
    num: u64,
    string: String,
}

impl Ptype for RedisData {
    fn regen(&mut self) {
        self.string = tools::random_string(self.size, self.num);
    }

    fn parse(seed: usize, size: usize, num: u64, _: &BTreeMap<String, Value>) -> CResult<Self> {
        Ok(RedisData {
            size: size,
            num: num,
            string: tools::seeded_string(size, seed),
        })
    }
}

#[derive(Clone)]
enum Command {
    Get(Param),
    Hget(Param, Param),
    Set(Param, Param, Option<Param>),
    Hset(Param, Param, Param),
    Del(Param),
    Expire(Param, Param),
    Incr(Param),
    Decr(Param),
    Append(Param, Param),
    Prepend(Param, Param),
    Eval(String, Vec<Param>),
    Evalsha(String, Vec<Param>),
}

impl Command {
    fn gen(&mut self) -> Vec<u8> {
        match *self {
            Command::Get(ref mut p1) => {
                p1.regen();
                gen::get(p1.value.string.as_str()).into_bytes()
            }
            Command::Hget(ref mut p1, ref mut p2) => {
                p1.regen();
                p2.regen();
                gen::hget(p1.value.string.as_str(), p2.value.string.as_str()).into_bytes()
            }
            Command::Set(ref mut p1, ref mut p2, ref mut p3) => {
                p1.regen();
                p2.regen();
                if let Some(p3_val) = p3 {
                    gen::set(
                        p1.value.string.as_str(),
                        p2.value.string.as_str(),
                        Some(p3_val.value.string.as_str()),
                    ).into_bytes()
                } else {
                    gen::set(p1.value.string.as_str(), p2.value.string.as_str(), None).into_bytes()
                }
            }
            Command::Hset(ref mut p1, ref mut p2, ref mut p3) => {
                p1.regen();
                p2.regen();
                p3.regen();
                gen::hset(
                    p1.value.string.as_str(),
                    p2.value.string.as_str(),
                    p3.value.string.as_str(),
                ).into_bytes()
            }
            Command::Del(ref mut p1) => {
                p1.regen();
                gen::del(p1.value.string.as_str()).into_bytes()
            }
            Command::Expire(ref mut p1, ref mut p2) => {
                p1.regen();
                gen::expire(
                    p1.value.string.as_str(),
                    p2.value.string.as_str().parse().unwrap(),
                ).into_bytes()
            }
            Command::Incr(ref mut p1) => {
                p1.regen();
                gen::incr(p1.value.string.as_str()).into_bytes()
            }
            Command::Decr(ref mut p1) => {
                p1.regen();
                gen::decr(p1.value.string.as_str()).into_bytes()
            }
            Command::Append(ref mut p1, ref mut p2) => {
                p1.regen();
                gen::append(p1.value.string.as_str(), p2.value.string.as_str()).into_bytes()
            }
            Command::Prepend(ref mut p1, ref mut p2) => {
                p1.regen();
                gen::prepend(p1.value.string.as_str(), p2.value.string.as_str()).into_bytes()
            }
            Command::Eval(ref p1, ref mut p2) => {
                let keys = regen_keys(p2);
                gen::eval(p1.as_str(), keys).into_bytes()
            }
            Command::Evalsha(ref p1, ref mut p2) => {
                let keys = regen_keys(p2);
                gen::evalsha(p1.as_str(), keys).into_bytes()
            }
        }
    }
}

fn regen_keys(keys: &mut Vec<Param>) -> Vec<&str> {
    keys.iter_mut()
        .map(|key| {
            key.regen();
            key.value.string.as_str()
        })
        .collect()
}

pub struct RedisParse;

struct RedisParseFactory {
    flush: bool,
    database: u32,
    preload_scripts: Vec<String>,
}

impl ProtocolGen for Command {
    fn generate_message(&mut self) -> Vec<u8> {
        self.gen()
    }

    fn method(&self) -> &str {
        match *self {
            Command::Get(_) => "get",
            Command::Set(_, _, _) => "set",
            Command::Hget(_, _) => "hget",
            Command::Hset(_, _, _) => "hset",
            Command::Del(_) => "del",
            Command::Expire(_, _) => "expire",
            Command::Incr(_) => "incr",
            Command::Decr(_) => "decr",
            Command::Append(_, _) => "append",
            Command::Prepend(_, _) => "prepend",
            Command::Eval(_, _) => "eval",
            Command::Evalsha(_, _) => "evalsha",
        }
    }

    fn boxed(&self) -> Box<ProtocolGen> {
        Box::new(self.clone())
    }
}

impl ProtocolParseFactory for RedisParseFactory {
    fn new(&self) -> Box<ProtocolParse> {
        Box::new(RedisParse)
    }

    fn prepare(&self) -> CResult<Vec<Vec<u8>>> {
        let mut ops = Vec::new();
        if self.flush {
            ops.push(gen::flushall().into_bytes());
            ops.push(gen::select(&self.database).into_bytes());
        }

        if !self.preload_scripts.is_empty() {
            ops.push(gen::script_flush().into_bytes());
        }

        ops.push(gen::select(&self.database).into_bytes());

        for script in self.preload_scripts.iter() {
            ops.push(gen::script_load(script.as_str()).into_bytes());
        }

        Ok(ops)
    }

    fn name(&self) -> &str {
        "redis"
    }
}

impl ProtocolParse for RedisParse {
    fn parse(&self, bytes: &[u8]) -> ParsedResponse {
        if let Ok(s) = str::from_utf8(bytes) {
            parse::parse_response(s)
        } else {
            ParsedResponse::Invalid
        }
    }
}

/// Load the redis benchmark configuration from the config toml and command line arguments
pub fn load_config(table: &BTreeMap<String, Value>, matches: &Matches) -> CResult<ProtocolConfig> {
    let mut ws = Vec::new();

    let database = table
        .get("general")
        .and_then(|k| k.as_table())
        .and_then(|k| k.get("database"))
        .and_then(|k| k.as_integer())
        .unwrap_or(0) as u32;

    if let Some(&Value::Array(ref workloads)) = table.get("workload") {
        let mut preload_scripts = Vec::new();

        for workload in workloads.iter() {
            if let Value::Table(ref workload) = *workload {
                let method = workload
                    .get("method")
                    .and_then(|k| k.as_str());
                let script = workload
                    .get("script-body")
                    .and_then(|k| k.as_str());

                if let (Some("evalsha"), Some(script)) = (method, script) {
                    preload_scripts.push(script.to_owned());
                }

                ws.push(extract_workload(workload)?);
            } else {
                return Err("workload must be table".to_owned());
            }
        }

        let proto = Arc::new(RedisParseFactory {
            flush: matches.opt_present("flush"),
            database: database,
            preload_scripts: preload_scripts,
        });

        Ok(ProtocolConfig {
            protocol: proto,
            workloads: ws,
            warmups: Vec::new(), // todo: write warmup extraction logic
        })
    } else {
        Err("no workloads specified".to_owned())
    }
}

fn extract_workload(workload: &BTreeMap<String, Value>) -> CResult<BenchmarkWorkload> {
    let rate = workload
        .get("rate")
        .and_then(|k| k.as_integer())
        .unwrap_or(0);

    let method = workload
        .get("method")
        .and_then(|k| k.as_str())
        .unwrap_or("get")
        .to_owned();

    let name = workload
        .get("name")
        .and_then(|k| k.as_str())
        .unwrap_or_else(|| method.as_str())
        .to_owned();

    if let Some(&Value::Array(ref params)) = workload.get("parameter") {
        let mut ps = Vec::new();
        for (i, param) in params.iter().enumerate() {
            match *param {
                Value::Table(ref parameter) => {
                    let p = cfgtypes::extract_parameter(i, parameter)?;
                    ps.push(p);
                }
                _ => {
                    return Err("malformed config: a parameter must be a struct".to_owned());
                }
            }
        }
        let cmd = match method.as_str() {
            "get" if ps.len() == 1 => Command::Get(ps[0].clone()),
            "hget" if ps.len() == 2 => Command::Hget(ps[0].clone(), ps[1].clone()),
            "set" if ps.len() == 2 => Command::Set(ps[0].clone(), ps[1].clone(), None),
            "set" if ps.len() == 3 => {
                Command::Set(ps[0].clone(), ps[1].clone(), Some(ps[2].clone()))
            }
            "hset" if ps.len() == 3 => Command::Hset(ps[0].clone(), ps[1].clone(), ps[2].clone()),
            "del" if ps.len() == 1 => Command::Del(ps[0].clone()),
            "expire" if ps.len() == 2 => Command::Expire(ps[0].clone(), ps[1].clone()),
            "incr" if ps.len() == 1 => Command::Incr(ps[0].clone()),
            "decr" if ps.len() == 1 => Command::Decr(ps[0].clone()),
            "append" if ps.len() == 2 => Command::Append(ps[0].clone(), ps[1].clone()),
            "prepend" if ps.len() == 1 => Command::Prepend(ps[0].clone(), ps[1].clone()),
            "eval" if ps.len() >= 4 && ps.len() % 2 == 0 && workload.get("script-body").is_some() => {
                let body = workload
                    .get("script-body")
                    .and_then(|k| k.as_str())
                    .unwrap();

                Command::Eval(
                    body.to_owned(),
                    ps.iter().skip(1).map(|v| v.clone()).collect(),
                )
            }
            "evalsha" if ps.len() >= 4 && ps.len() % 2 == 0 && workload.get("script-body").is_some() => {
                let body = workload
                    .get("script-body")
                    .and_then(|k| k.as_str())
                    .unwrap();

                let mut hasher = Sha1::new();
                hasher.input_str(body);

                Command::Evalsha(
                    hasher.result_str(),
                    ps.iter().skip(1).map(|v| v.clone()).collect(),
                )
            }
            "eval" | "evalsha" if workload.get("script-body").is_none() => {
                return Err(format!(
                    "workload.script-body is mandatory for method {}",
                    method
                ));
            }
            "eval" | "evalsha" => {
                return Err(format!(
                    "invalid number of params ({}) for method {}",
                    ps.len(),
                    method
                ));
            }
            "get" | "set" | "hset" | "hget" | "del" | "expire" | "incr" | "decr" | "append" |
            "prepend" => {
                return Err(format!(
                    "invalid number of params ({}) for method {}",
                    ps.len(),
                    method
                ));
            }
            _ => return Err(format!("invalid command: {}", method)),
        };

        Ok(BenchmarkWorkload::new(name, rate as usize, Box::new(cmd)))
    } else {
        Err("malformed config: 'parameter' must be an array".to_owned())
    }
}
