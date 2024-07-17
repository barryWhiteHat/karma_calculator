use itertools::Itertools;
use phantom_zone::{
    aggregate_server_key_shares, gen_client_key, gen_server_key_share, set_common_reference_seed,
    set_parameter_set, ClientKey, Encryptor, ParameterSelector,
};
use rand::{thread_rng, RngCore};
use std::borrow::Cow;

use rocket::tokio::sync::Mutex;
use rocket::{get, launch, post, routes};
use rocket::{Responder, State};

use rocket::serde::json::{json, Json, Value};
use rocket::serde::{Deserialize, Serialize};

// The type to represent the ID of a message.
type UserId = usize;
type ServerKeyShare = Vec<u8>;
type Cipher = Vec<u8>;

enum Registration {
    IDAcquired,
    KeySubmitted { sks: ServerKeyShare, cipher: Cipher },
}

struct RegisteredUser {
    name: String,
    registration: Registration,
}

// We're going to store all of the messages here. No need for a DB.
type UserList = Mutex<Vec<RegisteredUser>>;
type Users<'r> = &'r State<UserList>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(crate = "rocket::serde")]
struct Parameters {
    seed: [u8; 32],
}

impl Parameters {
    fn new(seed: [u8; 32]) -> Self {
        Self { seed }
    }
}

// TODO: how should the user get this value before everyone registered?
const TOTAL_USERS: usize = 3;

struct User {
    name: String,
    // step 0: get seed
    seed: Option<[u8; 32]>,
    // step 0.5: gen client key
    ck: Option<ClientKey>,
    // step 1: get userID
    id: Option<UserId>,
    // step 2: assign scores
    scores: Option<[u8; 4]>,
    // step 3: gen key and cipher
    server_key: Option<ServerKeyShare>,
    cipher: Option<Cipher>,
}

impl User {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            ck: None,
            id: None,
            seed: None,
            scores: None,
            server_key: None,
            cipher: None,
        }
    }

    fn assign_seed(&mut self, seed: [u8; 32]) -> &mut Self {
        self.seed = Some(seed);
        self
    }

    fn set_seed(&self) {
        set_common_reference_seed(self.seed.unwrap());
    }

    fn gen_client_key(&mut self) -> &mut Self {
        self.ck = Some(gen_client_key());
        self
    }

    fn set_id(&mut self, id: usize) -> &mut Self {
        self.id = Some(id);
        self
    }
    fn assign_scores(&mut self, scores: &[u8; 4]) -> &mut Self {
        self.scores = Some(scores.clone());
        self
    }

    fn gen_cipher(&mut self) -> &mut Self {
        // let score: [u8; 4] = self.scores.unwrap();
        // let ck: ClientKey = self.ck.clone().unwrap();
        // let cipher = ck.encrypt(&score);
        // self.cipher = Some(bincode::serialize(&cipher).unwrap());
        self
    }

    fn gen_server_key_share(&mut self) -> &mut Self {
        let server_key =
            gen_server_key_share(self.id.unwrap(), TOTAL_USERS, &self.ck.clone().unwrap());
        self.server_key = Some(bincode::serialize(&server_key).unwrap());
        self
    }
}

#[derive(Serialize, Deserialize)]
#[serde(crate = "rocket::serde")]
struct CipherSubmission<'r> {
    user_id: UserId,
    cipher_text: Cow<'r, Cipher>,
}

#[get("/world")]
fn world() -> &'static str {
    "Hello, world!"
}

#[get("/param")]
fn get_param(param: &State<Parameters>) -> Json<[u8; 32]> {
    Json(param.seed)
}

#[derive(Serialize, Deserialize)]
#[serde(crate = "rocket::serde")]
struct RegistrationOut {
    name: String,
    user_id: usize,
}

/// A user registers a name and get an ID
#[post("/register", data = "<name>")]
async fn register(name: &str, users: Users<'_>) -> Json<RegistrationOut> {
    let mut users = users.lock().await;
    let user_id = users.len();
    let user = RegisteredUser {
        name: name.to_string(),
        registration: Registration::IDAcquired,
    };
    users.push(user);
    Json(RegistrationOut {
        name: name.to_string(),
        user_id,
    })
}

/// The user submits the ciphertext
#[post("/submit", data = "<cipher>")]
async fn submit(cipher: Json<CipherSubmission<'_>>, users: Users<'_>) -> Value {
    let mut users = users.lock().await;
    let user_id = cipher.user_id;
    if users.len() <= user_id {
        return json!({ "status": "fail", "reason": format!("{user_id} hasn't registered yet") });
    }
    // users[user_id].cipher = Some(cipher.cipher_text.to_vec());
    json!({ "status": "ok", "user_id": user_id })
}

/// The admin runs the fhe computation
#[post("/run")]
async fn run(users: Users<'_>) -> Value {
    let users = users.lock().await;
    println!("checking if we have all user submissions");
    if users.len() < TOTAL_USERS {
        return json!( {"status": "fail", "reason":"some users haven't registered yet"});
    }
    // for (user_id, user) in users.iter().enumerate() {
    //     if user.cipher.is_none() {
    //         return json!( {"status": "fail", "reason":format!("user {user_id} hasn't submit cipher yet")});
    //     }
    // }

    // println!("derive server key");

    // let server_key_shares = users
    //     .iter()
    //     .map(|u| bincode::deserialize(&u.cipher.clone().unwrap()).unwrap())
    //     .collect_vec();

    // let server_key = aggregate_server_key_shares(&server_key_shares);
    // server_key.set_server_key();

    json!({ "status": "ok"})
}

#[launch]
fn rocket() -> _ {
    let mut seed = [0u8; 32];
    thread_rng().fill_bytes(&mut seed);

    set_parameter_set(ParameterSelector::NonInteractiveLTE4Party);
    set_common_reference_seed(seed);

    rocket::build()
        .manage(UserList::new(vec![]))
        .manage(Parameters::new(seed))
        .mount("/hello", routes![world])
        .mount("/", routes![get_param, register, submit, run])
}

#[cfg(test)]
mod tests {
    use super::*;
    use rocket::local::blocking::Client;

    #[test]
    fn hello() {
        let client = Client::tracked(super::rocket()).unwrap();
        let response = client.get("/hello/world").dispatch();
        assert_eq!(response.into_string(), Some("Hello, world!".into()));
    }

    #[test]
    fn full_flow() {
        let client = Client::tracked(super::rocket()).unwrap();

        let mut users = vec![User::new("Barry"), User::new("Justin"), User::new("Brian")];

        // Acquire seeds
        for user in users.iter_mut() {
            let seed = client
                .get("/param")
                .dispatch()
                .into_json::<[u8; 32]>()
                .expect("exists");
            user.assign_seed(seed);
            user.gen_client_key();
        }

        // Register
        for user in users.iter_mut() {
            let out = client
                .post("/register")
                .dispatch()
                .into_json::<RegistrationOut>()
                .expect("exists");
            user.set_id(out.user_id);
        }
    }
}
