use serde::Serialize;

pub struct UserData {
    pub accounts: Vec<Account>,
}

#[derive(Serialize)]
pub struct Account {
    pub account_id: i32,
    pub agreement: String,
    pub portfolio: String,
    #[serde(rename = "type")]
    pub portfolio_type: String,
    pub exchanges: Vec<String>,
    pub boards: Vec<String>,
}

impl UserData {
    pub fn find_account(&self, account_id: i32, board: &str) -> Option<&Account> {
        self.accounts.iter().find(|account| {
            account.account_id == account_id && account.boards.contains(&board.to_string())
        })
    }
}