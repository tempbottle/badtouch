use hlua;
use hlua::{AnyLuaValue, AnyHashableLuaValue, AnyLuaString};
use hlua::AnyLuaValue::LuaString;
use errors::{Result, ResultExt};
use json;

use md5;
use sha1;
use sha2;
use sha3::{self, Digest};
use base64;

use reqwest;
use ldap3;
use mysql;
use rand;
use rand::Rng;

use std::thread;
use std::time::Duration;
use std::process::Command;
use std::collections::HashMap;
use ctx::State;
use http::RequestOptions;
use html;


fn byte_array(bytes: AnyLuaValue) -> Result<Vec<u8>> {
    match bytes {
        AnyLuaValue::LuaAnyString(bytes) => Ok(bytes.0),
        AnyLuaValue::LuaString(bytes) => Ok(bytes.into_bytes()),
        AnyLuaValue::LuaArray(bytes) => {
            Ok(bytes.into_iter()
                .map(|num| match num.1 {
                    AnyLuaValue::LuaNumber(num) if num <= 255.0 && num >= 0.0 && (num % 1.0 == 0.0) =>
                            Ok(num as u8),
                    AnyLuaValue::LuaNumber(num) =>
                            Err(format!("number is out of range: {:?}", num).into()),
                    _ => Err(format!("unexpected type: {:?}", num).into()),
                })
                .collect::<Result<_>>()?)
        },
        _ => return Err(format!("invalid type: {:?}", bytes).into()),
    }
}

pub fn lua_bytes(bytes: &[u8]) -> AnyLuaValue {
    let bytes = AnyLuaString(bytes.to_vec());
    AnyLuaValue::LuaAnyString(bytes)
}


pub fn base64_decode(lua: &mut hlua::Lua, state: State) {
    lua.set("base64_decode", hlua::function1(move |bytes: String| -> Result<AnyLuaValue> {
        let bytes = match base64::decode(&bytes) {
            Ok(bytes) => bytes,
            Err(err) => return Err(state.set_error(err.into())),
        };

        Ok(lua_bytes(&bytes))
    }))
}

pub fn base64_encode(lua: &mut hlua::Lua, state: State) {
    lua.set("base64_encode", hlua::function1(move |bytes: AnyLuaValue| -> Result<String> {
        let bytes = match byte_array(bytes) {
            Ok(bytes) => bytes,
            Err(err) => return Err(state.set_error(err)),
        };

        Ok(base64::encode(&bytes))
    }))
}

pub fn execve(lua: &mut hlua::Lua, state: State) {
    lua.set("execve", hlua::function2(move |prog: String, args: Vec<AnyLuaValue>| -> Result<i32> {
        let args: Vec<_> = args.into_iter()
                    .flat_map(|x| match x {
                        LuaString(x) => Some(x),
                        _ => None, // TODO: error
                    })
                    .collect();

        let status = match Command::new(prog)
                        .args(&args)
                        .status()
                        .chain_err(|| "failed to spawn program") {
            Ok(status) => status,
            Err(err) => return Err(state.set_error(err)),
        };

        let code = match status.code() {
            Some(code) => code,
            None => return Err(state.set_error("process didn't return exit code".into())),
        };

        Ok(code)
    }))
}

pub fn hex(lua: &mut hlua::Lua, state: State) {
    lua.set("hex", hlua::function1(move |bytes: AnyLuaValue| -> Result<String> {
        let bytes = match byte_array(bytes) {
            Ok(bytes) => bytes,
            Err(err) => return Err(state.set_error(err)),
        };

        let mut out = String::new();

        for b in bytes {
            out += &format!("{:02x}", b);
        }

        Ok(out)
    }))
}

pub fn html_select(lua: &mut hlua::Lua, state: State) {
    lua.set("html_select", hlua::function2(move |html: String, selector: String| -> Result<AnyLuaValue> {
        match html::html_select(&html, &selector) {
            Ok(x) => Ok(x.into()),
            Err(err) => Err(state.set_error(err)),
        }
    }))
}

pub fn html_select_list(lua: &mut hlua::Lua, state: State) {
    lua.set("html_select_list", hlua::function2(move |html: String, selector: String| -> Result<Vec<AnyLuaValue>> {
        match html::html_select_list(&html, &selector) {
            Ok(x) => Ok(x.into_iter().map(|x| x.into()).collect()),
            Err(err) => Err(state.set_error(err)),
        }
    }))
}

pub fn http_basic_auth(lua: &mut hlua::Lua, state: State) {
    lua.set("http_basic_auth", hlua::function3(move |url: String, user: String, password: String| -> Result<bool> {
        let client = reqwest::Client::new();

        let response = match client.get(&url)
                             .basic_auth(user, Some(password))
                             .send()
                             .chain_err(|| "http request failed") {
            Ok(response) => response,
            Err(err) => return Err(state.set_error(err)),
         };

        // println!("{:?}", response);
        // println!("{:?}", response.headers().get_raw("www-authenticate"));
        // println!("{:?}", response.status());

        let authorized = response.headers().get_raw("www-authenticate").is_none() &&
            response.status() != reqwest::StatusCode::Unauthorized;

        Ok(authorized)
    }))
}

pub fn http_mksession(lua: &mut hlua::Lua, state: State) {
    lua.set("http_mksession", hlua::function0(move || -> String {
        state.http_mksession()
    }))
}

pub fn http_request(lua: &mut hlua::Lua, state: State) {
    lua.set("http_request", hlua::function4(move |session: String, method: String, url: String, options: AnyLuaValue| -> Result<String> {
        let options = match RequestOptions::try_from(options)
                                .chain_err(|| "invalid request options") {
            Ok(options) => options,
            Err(err) => return Err(state.set_error(err)),
        };

        let id = state.http_request(&session, method, url, options);
        Ok(id)
    }))
}

pub fn http_send(lua: &mut hlua::Lua, state: State) {
    lua.set("http_send", hlua::function1(move |request: String| -> Result<HashMap<AnyHashableLuaValue, AnyLuaValue>> {
        let resp = match state.http_send(request) {
            Ok(resp) => resp,
            Err(err) => return Err(state.set_error(err)),
        };
        Ok(resp.into())
    }))
}

pub fn json_decode(lua: &mut hlua::Lua, state: State) {
    lua.set("json_decode", hlua::function1(move |x: String| -> Result<AnyLuaValue> {
        match json::decode(&x) {
            Ok(x) => Ok(x),
            Err(err) => Err(state.set_error(err)),
        }
    }))
}

pub fn json_encode(lua: &mut hlua::Lua, state: State) {
    lua.set("json_encode", hlua::function1(move |x: AnyLuaValue| -> Result<String> {
        match json::encode(x) {
            Ok(x) => Ok(x),
            Err(err) => Err(state.set_error(err)),
        }
    }))
}

pub fn last_err(lua: &mut hlua::Lua, state: State) {
    lua.set("last_err", hlua::function0(move || -> AnyLuaValue {
        match state.last_error() {
            Some(err) => AnyLuaValue::LuaString(err),
            None => AnyLuaValue::LuaNil,
        }
    }))
}

pub fn ldap_bind(lua: &mut hlua::Lua, state: State) {
    lua.set("ldap_bind", hlua::function3(move |url: String, dn: String, password: String| -> Result<bool> {
        let sock = match ldap3::LdapConn::new(&url)
                        .chain_err(|| "ldap connection failed") {
            Ok(sock) => sock,
            Err(err) => return Err(state.set_error(err)),
        };

        let result = match sock.simple_bind(&dn, &password)
                            .chain_err(|| "fatal error during simple_bind") {
            Ok(result) => result,
            Err(err) => return Err(state.set_error(err)),
        };

        // println!("{:?}", result);

        Ok(result.success().is_ok())
    }))
}

pub fn ldap_escape(lua: &mut hlua::Lua, _: State) {
    lua.set("ldap_escape", hlua::function1(move |s: String| -> String {
        ldap3::dn_escape(s).to_string()
    }))
}

pub fn ldap_search_bind(lua: &mut hlua::Lua, state: State) {
    lua.set("ldap_search_bind", hlua::function6(move |url: String, search_user: String, search_pw: String, base_dn: String, user: String, password: String| -> Result<bool> {

        let sock = match ldap3::LdapConn::new(&url)
                        .chain_err(|| "ldap connection failed") {
            Ok(sock) => sock,
            Err(err) => return Err(state.set_error(err)),
        };

        let result = match sock.simple_bind(&search_user, &search_pw)
                            .chain_err(|| "fatal error during simple_bind with search user") {
            Ok(result) => result,
            Err(err) => return Err(state.set_error(err)),
        };

        if !result.success().is_ok() {
            return Err("login with search user failed".into());
        }

        let search = format!("uid={}", ldap3::dn_escape(user));
        let result = match sock.search(&base_dn, ldap3::Scope::Subtree, &search, vec!["*"])
                            .chain_err(|| "fatal error during ldap search") {
            Ok(result) => result,
            Err(err) => return Err(state.set_error(err)),
        };

        let entries = match result.success()
                            .chain_err(|| "ldap search failed") {
            Ok(result) => result.0,
            Err(err) => return Err(state.set_error(err)),
        };

        // take the first result
        if let Some(entry) = entries.into_iter().next() {
            let entry = ldap3::SearchEntry::construct(entry);

            // we got the DN, try to login
            let result = match sock.simple_bind(&entry.dn, &password)
                                .chain_err(|| "fatal error during simple_bind") {
                Ok(result) => result,
                Err(err) => return Err(state.set_error(err)),
            };

            // println!("{:?}", result);

            Ok(result.success().is_ok())
        } else {
            return Ok(false);
        }
    }))
}

pub fn md5(lua: &mut hlua::Lua, state: State) {
    lua.set("md5", hlua::function1(move |bytes: AnyLuaValue| -> Result<AnyLuaValue> {
        let bytes = match byte_array(bytes) {
            Ok(bytes) => bytes,
            Err(err) => return Err(state.set_error(err)),
        };

        Ok(lua_bytes(&md5::Md5::digest(&bytes)))
    }))
}

pub fn mysql_connect(lua: &mut hlua::Lua, _state: State) {
    lua.set("mysql_connect", hlua::function4(move |host: String, port: u16, user: String, password: String| -> Result<bool> {
        let mut builder = mysql::OptsBuilder::new();
        builder.ip_or_hostname(Some(host))
               .tcp_port(port)
               .prefer_socket(false)
               .user(Some(user))
               .pass(Some(password));

        match mysql::Conn::new(builder) {
            Ok(_) => Ok(true),
            Err(_err) => {
                // TODO: err
                // println!("{:?}", _err);
                Ok(false)
            },
        }
    }))
}

fn format_lua(out: &mut String, x: &AnyLuaValue) {
    match *x {
        AnyLuaValue::LuaNil => out.push_str("null"),
        AnyLuaValue::LuaString(ref x) => out.push_str(&format!("{:?}", x)),
        AnyLuaValue::LuaNumber(ref x) => out.push_str(&format!("{:?}", x)),
        AnyLuaValue::LuaAnyString(ref x) => out.push_str(&format!("{:?}", x.0)),
        AnyLuaValue::LuaBoolean(ref x) => out.push_str(&format!("{:?}", x)),
        AnyLuaValue::LuaArray(ref x) => {
            out.push_str("{");
            let mut first = true;

            for &(ref k, ref v) in x {
                if !first {
                    out.push_str(", ");
                }

                let mut key = String::new();
                format_lua(&mut key, &k);

                let mut value = String::new();
                format_lua(&mut value, &v);

                out.push_str(&format!("{}: {}", key, value));

                first = false;
            }
            out.push_str("}");
        },
        AnyLuaValue::LuaOther => out.push_str("LuaOther"),
    }
}

pub fn print(lua: &mut hlua::Lua, _: State) {
    // this function doesn't print to the terminal safely
    // only use this for debugging
    lua.set("print", hlua::function1(move |val: AnyLuaValue| {
        // println!("{:?}", val);
        let mut out = String::new();
        format_lua(&mut out, &val);
        println!("{}", out);
    }))
}

pub fn rand(lua: &mut hlua::Lua, _: State) {
    lua.set("rand", hlua::function2(move |min: u32, max: u32| -> u32 {
        let mut rng = rand::thread_rng();
        (rng.next_u32() + min) % max
    }))
}

pub fn sha1(lua: &mut hlua::Lua, state: State) {
    lua.set("sha1", hlua::function1(move |bytes: AnyLuaValue| -> Result<AnyLuaValue> {
        let bytes = match byte_array(bytes) {
            Ok(bytes) => bytes,
            Err(err) => return Err(state.set_error(err)),
        };

        Ok(lua_bytes(&sha1::Sha1::digest(&bytes)))
    }))
}

pub fn sha2_256(lua: &mut hlua::Lua, state: State) {
    lua.set("sha2_256", hlua::function1(move |bytes: AnyLuaValue| -> Result<AnyLuaValue> {
        let bytes = match byte_array(bytes) {
            Ok(bytes) => bytes,
            Err(err) => return Err(state.set_error(err)),
        };

        Ok(lua_bytes(&sha2::Sha256::digest(&bytes)))
    }))
}

pub fn sha2_512(lua: &mut hlua::Lua, state: State) {
    lua.set("sha2_512", hlua::function1(move |bytes: AnyLuaValue| -> Result<AnyLuaValue> {
        let bytes = match byte_array(bytes) {
            Ok(bytes) => bytes,
            Err(err) => return Err(state.set_error(err)),
        };

        Ok(lua_bytes(&sha2::Sha512::digest(&bytes)))
    }))
}

pub fn sha3_256(lua: &mut hlua::Lua, state: State) {
    lua.set("sha3_256", hlua::function1(move |bytes: AnyLuaValue| -> Result<AnyLuaValue> {
        let bytes = match byte_array(bytes) {
            Ok(bytes) => bytes,
            Err(err) => return Err(state.set_error(err)),
        };

        Ok(lua_bytes(&sha3::Sha3_256::digest(&bytes)))
    }))
}

pub fn sha3_512(lua: &mut hlua::Lua, state: State) {
    lua.set("sha3_512", hlua::function1(move |bytes: AnyLuaValue| -> Result<AnyLuaValue> {
        let bytes = match byte_array(bytes) {
            Ok(bytes) => bytes,
            Err(err) => return Err(state.set_error(err)),
        };

        Ok(lua_bytes(&sha3::Sha3_512::digest(&bytes)))
    }))
}

pub fn sleep(lua: &mut hlua::Lua, _: State) {
    lua.set("sleep", hlua::function1(move |n: i32| {
        thread::sleep(Duration::from_secs(n as u64));
        0
    }))
}
