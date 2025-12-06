use solana_sdk::{
    bs58,
    signature::{Keypair, keypair_from_seed},
};

pub struct PuppetMaster {
    master: Keypair,
    puppets: Vec<Keypair>,
}

impl PuppetMaster {
    pub fn new(wallet_amount: u8) -> Self {
        let puppets: Vec<_> = (0..wallet_amount).map(|_| Keypair::new()).collect();

        Self {
            master: Keypair::new(),
            puppets,
        }
    }

    pub fn master(&self) -> &Keypair {
        &self.master
    }

    pub fn puppets(&self) -> &[Keypair] {
        &self.puppets
    }

    pub fn encode(&self) -> String {
        let mut buffer = String::new();

        buffer.push_str(&format!("{}\n", self.master().to_base58_string()));

        self.puppets().iter().for_each(|keypair| {
            buffer.push_str(&format!("{}\n", keypair.to_base58_string()));
        });

        buffer
    }

    pub fn save_to_file(&self, file_location: &str) {
        let data = self.encode();

        let _ = std::fs::write(file_location, data);
    }

    fn keypair_from_str(str: &str) -> Result<Keypair, &'static str> {
        let Ok(bytes) = bs58::decode(str.as_bytes()).into_vec() else {
            return Err("Not BASE58 encoded keypair");
        };

        let Ok(keypair) = keypair_from_seed(&bytes) else {
            return Err("Not valid keypair");
        };

        Ok(keypair)
    }

    pub fn decode(path: &str) -> Result<Self, &'static str> {
        let Ok(data) = std::fs::read_to_string(path) else {
            return Err("Not found config");
        };

        let mut iterator = data.lines();

        let Some(master) = iterator.next() else {
            return Err("Not found master wallet");
        };

        let Ok(master) = PuppetMaster::keypair_from_str(master) else {
            return Err("Not valid master keypair");
        };

        let mut puppets = vec![];

        for entry in iterator {
            let Ok(keypair) = PuppetMaster::keypair_from_str(entry) else {
                return Err("Not valid keypair");
            };

            puppets.push(keypair);
        }

        Ok(PuppetMaster { master, puppets })
    }
}
