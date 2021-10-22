use serde::de::DeserializeOwned;
use serde::Serialize;

use cosmwasm_std::{
    Binary, Deps, DepsMut, Env, MessageInfo,
    Response, StdResult, Api, Addr, Uint128,
    BankMsg, coins,
};

use cw2::set_contract_version;
use cw721::{ContractInfoResponse, CustomMsg, Cw721Execute, Cw721ReceiveMsg, Expiration};

use crate::error::ContractError;
use crate::msg::{ExecuteMsg, InstantiateMsg, MintMsg};
use crate::state::{Approval, Cw721Contract, TokenInfo, WhiteList};

// version info for migration info
const CONTRACT_NAME: &str = "nft";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

impl<'a, T, C> Cw721Contract<'a, T, C>
where
    T: Serialize + DeserializeOwned + Clone,
    C: CustomMsg,
{
    pub fn instantiate(
        &self,
        deps: DepsMut,
        _env: Env,
        _info: MessageInfo,
        msg: InstantiateMsg,
    ) -> StdResult<Response<C>> {
        set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

        let info = ContractInfoResponse {
            name: msg.name,
            symbol: msg.symbol,
        };
        let white_list = WhiteList {
            addresses: map_validate(deps.api, &msg.white_list)?,
        };
        let admin = Addr::unchecked(_info.sender);

        self.contract_info.save(deps.storage, &info)?;
        self.max_tokens.save(deps.storage, &msg.max_tokens)?;
        self.white_list.save(deps.storage, &white_list)?;
        self.admin.save(deps.storage, &admin)?;
        self.is_presale.save(deps.storage, &true)?;
        self.minting_fee.save(deps.storage, &msg.minting_fee)?;
        //let minter = deps.api.addr_validate(&msg.minter)?;
        //self.minter.save(deps.storage, &minter)?;
        Ok(Response::default())
    }

    pub fn execute(
        &self,
        deps: DepsMut,
        env: Env,
        info: MessageInfo,
        msg: ExecuteMsg<T>,
    ) -> Result<Response<C>, ContractError> {
        match msg {
            ExecuteMsg::Mint(msg) => self.mint(deps, env, info, msg),
            ExecuteMsg::Approve {
                spender,
                token_id,
                expires,
            } => self.approve(deps, env, info, spender, token_id, expires),
            ExecuteMsg::Revoke { spender, token_id } => {
                self.revoke(deps, env, info, spender, token_id)
            }
            ExecuteMsg::ApproveAll { operator, expires } => {
                self.approve_all(deps, env, info, operator, expires)
            }
            ExecuteMsg::RevokeAll { operator } => self.revoke_all(deps, env, info, operator),
            ExecuteMsg::TransferNft {
                recipient,
                token_id,
            } => self.transfer_nft(deps, env, info, recipient, token_id),
            ExecuteMsg::SendNft {
                contract,
                token_id,
                msg,
            } => self.send_nft(deps, env, info, contract, token_id, msg),
            ExecuteMsg::UpdateWhiteList {
                addresses
            } => self.execute_update_white_list(deps, env, info, addresses),
            ExecuteMsg::SetPresaleStatus(is_presale) => {
                self.set_presale_status(deps, env, info, is_presale)
            },
            ExecuteMsg::WithdrawBalance(amount) => {
                self.withdraw_balance(deps, env, info, amount)
            }
        }
    }
}

// TODO pull this into some sort of trait extension??
impl<'a, T, C> Cw721Contract<'a, T, C>
where
    T: Serialize + DeserializeOwned + Clone,
    C: CustomMsg,
{
    pub fn mint(
        &self,
        deps: DepsMut,
        env: Env,
        info: MessageInfo,
        msg: MintMsg<T>,
    ) -> Result<Response<C>, ContractError> {
        //let minter = self.minter.load(deps.storage)?;

        //if info.sender != minter {
        //    return Err(ContractError::Unauthorized {});
        //}

        let minter = &info.sender;

        let is_presale = self.is_presale.load(deps.storage)?;
        if is_presale  {
            let white_list = self.white_list.load(deps.storage)?;
            let is_white_listed = white_list.is_white_listed(minter.as_ref());
            if !is_white_listed {
                return Err(ContractError::Unauthorized {});
            }
        }

        // Check if maximum token number reached
        if self.token_count.load(deps.storage)? == self.max_tokens.load(deps.storage)? {
            return Err(ContractError::MaxTokensReached {});
        }

        // create the token
        let token = TokenInfo {
            //owner: deps.api.addr_validate(&msg.owner)?,
            owner: info.sender.clone(),
            approvals: vec![],
            token_uri: msg.token_uri,
            extension: msg.extension,
        };
        self.tokens
            .update(deps.storage, &msg.token_id, |old| match old {
                Some(_) => Err(ContractError::Claimed {}),
                None => Ok(token),
            })?;

        self.increment_tokens(deps.storage)?;

        if info.funds.len() == 0 {
            return Err(ContractError::InsufficientPayment {});
        }
        let coin = &info.funds[0];
        let minting_fee = self.minting_fee.load(deps.storage)?;
        if coin.amount < minting_fee {
            return Err(ContractError::InsufficientPayment {});
        } 

        // Querier guarantees to returns up-to-date data, including funds sent in this handle message
        // https://github.com/CosmWasm/wasmd/blob/master/x/wasm/internal/keeper/keeper.go#L185-L192
        let contract_addr = &env.contract.address;
        let balance = deps.querier.query_all_balances(contract_addr)?;
        println!("token amount is {:?}", balance);

        let mut contract_balance = "".to_string();
        if balance.len() > 0 {
            contract_balance = balance[0].amount.to_string();
        }

        Ok(Response::new()
            .add_attribute("action", "mint")
            .add_attribute("minter", minter)
            .add_attribute("token_id", msg.token_id)
            .add_attribute("contract_balance", contract_balance))
    }

    pub fn execute_update_white_list(
        &self,
        deps: DepsMut,
        _env: Env,
        info: MessageInfo,
        addresses: Vec<String>,
    ) -> Result<Response<C>, ContractError> {
        let admin = self.admin.load(deps.storage)?;
        let mut cfg = self.white_list.load(deps.storage)?;

        if admin != info.sender {
            Err(ContractError::Unauthorized {})
        } else {
            cfg.addresses = map_validate(deps.api, &addresses)?;
            self.white_list.save(deps.storage, &cfg)?;

            let res = Response::new().add_attribute("action", "update_white_list");
            Ok(res)
        }
    }

    pub fn set_presale_status(
        &self,
        deps: DepsMut,
        _env: Env,
        info: MessageInfo,
        is_presale: bool,
    ) -> Result<Response<C>, ContractError> {
        let admin = self.admin.load(deps.storage)?;
        
        if admin != info.sender {
            Err(ContractError::Unauthorized {})
        } else {
            self.is_presale.save(deps.storage, &is_presale)?;

            let res = Response::new()
                .add_attribute("action", "set_presale_status")
                .add_attribute("presale_status", is_presale.to_string());
            Ok(res)
        }
    }

    pub fn withdraw_balance(
        &self,
        deps: DepsMut,
        env: Env,
        _info: MessageInfo,
        withdraw_amount: Uint128,
    ) -> Result<Response<C>, ContractError> {

        let contract_addr = &env.contract.address;
        let balance = deps.querier.query_all_balances(contract_addr)?;

        if balance.len() == 0 {
            return Err(ContractError::InsufficientBalance {});
        }

        let available_amount = balance[0].amount;
        if withdraw_amount > available_amount {
            return Err(ContractError::InsufficientBalance {});
        }

        let admin_address = self.admin.load(deps.storage)?;
 
        let r = Response::new()
            .add_message(BankMsg::Send {
                to_address: admin_address.to_string(),
                amount: coins(withdraw_amount.into(), "uluna"),
            });

        Ok(r)
    }
}

// validate addresses
pub fn map_validate(api: &dyn Api, addresses: &[String]) -> StdResult<Vec<Addr>> {
    addresses.iter().map(|addr| api.addr_validate(&addr)).collect()
}



impl<'a, T, C> Cw721Execute<T, C> for Cw721Contract<'a, T, C>
where
    T: Serialize + DeserializeOwned + Clone,
    C: CustomMsg,
{
    type Err = ContractError;

    fn transfer_nft(
        &self,
        deps: DepsMut,
        env: Env,
        info: MessageInfo,
        recipient: String,
        token_id: String,
    ) -> Result<Response<C>, ContractError> {
        self._transfer_nft(deps, &env, &info, &recipient, &token_id)?;

        Ok(Response::new()
            .add_attribute("action", "transfer_nft")
            .add_attribute("sender", info.sender)
            .add_attribute("recipient", recipient)
            .add_attribute("token_id", token_id))
    }

    fn send_nft(
        &self,
        deps: DepsMut,
        env: Env,
        info: MessageInfo,
        contract: String,
        token_id: String,
        msg: Binary,
    ) -> Result<Response<C>, ContractError> {
        // Transfer token
        self._transfer_nft(deps, &env, &info, &contract, &token_id)?;

        let send = Cw721ReceiveMsg {
            sender: info.sender.to_string(),
            token_id: token_id.clone(),
            msg,
        };

        // Send message
        Ok(Response::new()
            .add_message(send.into_cosmos_msg(contract.clone())?)
            .add_attribute("action", "send_nft")
            .add_attribute("sender", info.sender)
            .add_attribute("recipient", contract)
            .add_attribute("token_id", token_id))
    }

    fn approve(
        &self,
        deps: DepsMut,
        env: Env,
        info: MessageInfo,
        spender: String,
        token_id: String,
        expires: Option<Expiration>,
    ) -> Result<Response<C>, ContractError> {
        self._update_approvals(deps, &env, &info, &spender, &token_id, true, expires)?;

        Ok(Response::new()
            .add_attribute("action", "approve")
            .add_attribute("sender", info.sender)
            .add_attribute("spender", spender)
            .add_attribute("token_id", token_id))
    }

    fn revoke(
        &self,
        deps: DepsMut,
        env: Env,
        info: MessageInfo,
        spender: String,
        token_id: String,
    ) -> Result<Response<C>, ContractError> {
        self._update_approvals(deps, &env, &info, &spender, &token_id, false, None)?;

        Ok(Response::new()
            .add_attribute("action", "revoke")
            .add_attribute("sender", info.sender)
            .add_attribute("spender", spender)
            .add_attribute("token_id", token_id))
    }

    fn approve_all(
        &self,
        deps: DepsMut,
        env: Env,
        info: MessageInfo,
        operator: String,
        expires: Option<Expiration>,
    ) -> Result<Response<C>, ContractError> {
        // reject expired data as invalid
        let expires = expires.unwrap_or_default();
        if expires.is_expired(&env.block) {
            return Err(ContractError::Expired {});
        }

        // set the operator for us
        let operator_addr = deps.api.addr_validate(&operator)?;
        self.operators
            .save(deps.storage, (&info.sender, &operator_addr), &expires)?;

        Ok(Response::new()
            .add_attribute("action", "approve_all")
            .add_attribute("sender", info.sender)
            .add_attribute("operator", operator))
    }

    fn revoke_all(
        &self,
        deps: DepsMut,
        _env: Env,
        info: MessageInfo,
        operator: String,
    ) -> Result<Response<C>, ContractError> {
        let operator_addr = deps.api.addr_validate(&operator)?;
        self.operators
            .remove(deps.storage, (&info.sender, &operator_addr));

        Ok(Response::new()
            .add_attribute("action", "revoke_all")
            .add_attribute("sender", info.sender)
            .add_attribute("operator", operator))
    }
}

// helpers
impl<'a, T, C> Cw721Contract<'a, T, C>
where
    T: Serialize + DeserializeOwned + Clone,
    C: CustomMsg,
{
    pub fn _transfer_nft(
        &self,
        deps: DepsMut,
        env: &Env,
        info: &MessageInfo,
        recipient: &str,
        token_id: &str,
    ) -> Result<TokenInfo<T>, ContractError> {
        let mut token = self.tokens.load(deps.storage, &token_id)?;
        // ensure we have permissions
        self.check_can_send(deps.as_ref(), env, info, &token)?;
        // set owner and remove existing approvals
        token.owner = deps.api.addr_validate(recipient)?;
        token.approvals = vec![];
        self.tokens.save(deps.storage, &token_id, &token)?;
        Ok(token)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn _update_approvals(
        &self,
        deps: DepsMut,
        env: &Env,
        info: &MessageInfo,
        spender: &str,
        token_id: &str,
        // if add == false, remove. if add == true, remove then set with this expiration
        add: bool,
        expires: Option<Expiration>,
    ) -> Result<TokenInfo<T>, ContractError> {
        let mut token = self.tokens.load(deps.storage, &token_id)?;
        // ensure we have permissions
        self.check_can_approve(deps.as_ref(), env, info, &token)?;

        // update the approval list (remove any for the same spender before adding)
        let spender_addr = deps.api.addr_validate(spender)?;
        token.approvals = token
            .approvals
            .into_iter()
            .filter(|apr| apr.spender != spender_addr)
            .collect();

        // only difference between approve and revoke
        if add {
            // reject expired data as invalid
            let expires = expires.unwrap_or_default();
            if expires.is_expired(&env.block) {
                return Err(ContractError::Expired {});
            }
            let approval = Approval {
                spender: spender_addr,
                expires,
            };
            token.approvals.push(approval);
        }

        self.tokens.save(deps.storage, &token_id, &token)?;

        Ok(token)
    }

    /// returns true iff the sender can execute approve or reject on the contract
    pub fn check_can_approve(
        &self,
        deps: Deps,
        env: &Env,
        info: &MessageInfo,
        token: &TokenInfo<T>,
    ) -> Result<(), ContractError> {
        // owner can approve
        if token.owner == info.sender {
            return Ok(());
        }
        // operator can approve
        let op = self
            .operators
            .may_load(deps.storage, (&token.owner, &info.sender))?;
        match op {
            Some(ex) => {
                if ex.is_expired(&env.block) {
                    Err(ContractError::Unauthorized {})
                } else {
                    Ok(())
                }
            }
            None => Err(ContractError::Unauthorized {}),
        }
    }

    /// returns true iff the sender can transfer ownership of the token
    fn check_can_send(
        &self,
        deps: Deps,
        env: &Env,
        info: &MessageInfo,
        token: &TokenInfo<T>,
    ) -> Result<(), ContractError> {
        // owner can send
        if token.owner == info.sender {
            return Ok(());
        }

        // any non-expired token approval can send
        if token
            .approvals
            .iter()
            .any(|apr| apr.spender == info.sender && !apr.is_expired(&env.block))
        {
            return Ok(());
        }

        // operator can send
        let op = self
            .operators
            .may_load(deps.storage, (&token.owner, &info.sender))?;
        match op {
            Some(ex) => {
                if ex.is_expired(&env.block) {
                    Err(ContractError::Unauthorized {})
                } else {
                    Ok(())
                }
            }
            None => Err(ContractError::Unauthorized {}),
        }
    }
}

