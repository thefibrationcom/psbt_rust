// SPDX-License-Identifier: CC0-1.0

//! PSBT serialization.
//!
//! Traits to serialize PSBT values to and from raw bytes
//! according to the BIP-174 specification.

use core::convert::{TryFrom, TryInto};
use core::fmt;

use bitcoin::bip32::{ChildNumber, Fingerprint, KeySource};
use bitcoin::consensus::{self, Decodable, Encodable};
use bitcoin::hashes::{self, hash160, ripemd160, sha256, sha256d, Hash};
use bitcoin::key::PublicKey;
use bitcoin::secp256k1::{self, XOnlyPublicKey};
use bitcoin::taproot::{
    ControlBlock, LeafVersion, TapLeafHash, TapNodeHash, TapTree, TaprootBuilder,
};
use bitcoin::{
    absolute, ecdsa, taproot, transaction, Amount, ScriptBuf, Sequence, Transaction, TxOut, Txid,
    VarInt, Witness,
};

use crate::error::write_err;
use crate::prelude::*;
use crate::sighash_type::PsbtSighashType;
use crate::version;

/// A trait for serializing a value as raw data for insertion into PSBT
/// key-value maps.
pub(crate) trait Serialize {
    /// Serialize a value as raw data.
    fn serialize(&self) -> Vec<u8>;
}

/// A trait for deserializing a value from raw data in PSBT key-value maps.
pub(crate) trait Deserialize: Sized {
    /// Deserialize a value from raw data.
    fn deserialize(bytes: &[u8]) -> Result<Self, Error>;
}

// Strictly speaking these do not need the prefix because the v0 versions are
// unused but we want to leave thoes in the code so the the files are close as
// possible to the original from bitcoin/miniscript repos.
v2_impl_psbt_de_serialize!(absolute::LockTime);
v2_impl_psbt_de_serialize!(Amount);
v2_impl_psbt_de_serialize!(Transaction);
v2_impl_psbt_de_serialize!(transaction::Version);
v2_impl_psbt_de_serialize!(TxOut);
v2_impl_psbt_de_serialize!(Witness);
v2_impl_psbt_de_serialize!(VarInt);
v2_impl_psbt_hash_de_serialize!(ripemd160::Hash);
v2_impl_psbt_hash_de_serialize!(sha256::Hash);
v2_impl_psbt_hash_de_serialize!(TapLeafHash);
v2_impl_psbt_hash_de_serialize!(TapNodeHash);
v2_impl_psbt_hash_de_serialize!(Txid);
v2_impl_psbt_hash_de_serialize!(hash160::Hash);
v2_impl_psbt_hash_de_serialize!(sha256d::Hash);

// taproot
v2_impl_psbt_de_serialize!(Vec<TapLeafHash>);

impl Serialize for ScriptBuf {
    fn serialize(&self) -> Vec<u8> { self.to_bytes() }
}

impl Deserialize for ScriptBuf {
    fn deserialize(bytes: &[u8]) -> Result<Self, Error> { Ok(Self::from(bytes.to_vec())) }
}

impl Serialize for PublicKey {
    fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        self.write_into(&mut buf).expect("vecs don't error");
        buf
    }
}

impl Deserialize for PublicKey {
    fn deserialize(bytes: &[u8]) -> Result<Self, Error> {
        PublicKey::from_slice(bytes).map_err(Error::InvalidPublicKey)
    }
}

impl Serialize for secp256k1::PublicKey {
    fn serialize(&self) -> Vec<u8> { self.serialize().to_vec() }
}

impl Deserialize for secp256k1::PublicKey {
    fn deserialize(bytes: &[u8]) -> Result<Self, Error> {
        secp256k1::PublicKey::from_slice(bytes).map_err(Error::InvalidSecp256k1PublicKey)
    }
}

impl Serialize for ecdsa::Signature {
    fn serialize(&self) -> Vec<u8> { self.to_vec() }
}

impl Deserialize for ecdsa::Signature {
    fn deserialize(bytes: &[u8]) -> Result<Self, Error> {
        // NB: Since BIP-174 says "the signature as would be pushed to the stack from
        // a scriptSig or witness" we should ideally use a consensus deserialization and do
        // not error on a non-standard values. However,
        //
        // 1) the current implementation of from_u32_consensus(`flag`) does not preserve
        // the sighash byte `flag` mapping all unknown values to EcdsaSighashType::All or
        // EcdsaSighashType::AllPlusAnyOneCanPay. Therefore, break the invariant
        // EcdsaSig::from_slice(&sl[..]).to_vec = sl.
        //
        // 2) This would cause to have invalid signatures because the sighash message
        // also has a field sighash_u32 (See BIP141). For example, when signing with non-standard
        // 0x05, the sighash message would have the last field as 0x05u32 while, the verification
        // would use check the signature assuming sighash_u32 as `0x01`.
        ecdsa::Signature::from_slice(bytes).map_err(|e| match e {
            ecdsa::Error::EmptySignature => Error::InvalidEcdsaSignature(e),
            ecdsa::Error::SighashType(err) => Error::NonStandardSighashType(err.0),
            ecdsa::Error::Secp256k1(..) => Error::InvalidEcdsaSignature(e),
            ecdsa::Error::Hex(..) => unreachable!("Decoding from slice, not hex"),
            _ => panic!("TODO: Handle non_exhaustive error"),
        })
    }
}

impl Serialize for KeySource {
    fn serialize(&self) -> Vec<u8> {
        let mut rv: Vec<u8> = Vec::with_capacity(key_source_len(self));

        rv.append(&mut self.0.to_bytes().to_vec());

        for cnum in self.1.into_iter() {
            rv.append(&mut consensus::serialize(&u32::from(*cnum)))
        }

        rv
    }
}

impl Deserialize for KeySource {
    fn deserialize(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() < 4 {
            return Err(Error::NotEnoughData);
        }

        let fprint: Fingerprint = bytes[0..4].try_into().expect("4 is the fingerprint length");
        let mut dpath: Vec<ChildNumber> = Default::default();

        let mut d = &bytes[4..];
        while !d.is_empty() {
            match u32::consensus_decode(&mut d) {
                Ok(index) => dpath.push(index.into()),
                Err(e) => return Err(e.into()),
            }
        }

        Ok((fprint, dpath.into()))
    }
}

impl Serialize for u32 {
    fn serialize(&self) -> Vec<u8> { consensus::serialize(&self) }
}

impl Deserialize for u32 {
    fn deserialize(bytes: &[u8]) -> Result<Self, Error> {
        let val: u32 = consensus::deserialize(bytes)?;
        Ok(val)
    }
}

impl Serialize for Sequence {
    fn serialize(&self) -> Vec<u8> { consensus::serialize(&self) }
}

impl Deserialize for Sequence {
    fn deserialize(bytes: &[u8]) -> Result<Self, Error> {
        let n: Sequence = consensus::deserialize(bytes)?;
        Ok(n)
    }
}

impl Serialize for absolute::Height {
    fn serialize(&self) -> Vec<u8> { consensus::serialize(&self.to_consensus_u32()) }
}

impl Deserialize for absolute::Height {
    fn deserialize(bytes: &[u8]) -> Result<Self, Error> {
        let n: u32 = consensus::deserialize(bytes)?;
        let lock = absolute::Height::from_consensus(n)?;
        Ok(lock)
    }
}

impl Serialize for absolute::Time {
    fn serialize(&self) -> Vec<u8> { consensus::serialize(&self.to_consensus_u32()) }
}

impl Deserialize for absolute::Time {
    fn deserialize(bytes: &[u8]) -> Result<Self, Error> {
        let n: u32 = consensus::deserialize(bytes)?;
        let lock = absolute::Time::from_consensus(n)?;
        Ok(lock)
    }
}

// partial sigs
impl Serialize for Vec<u8> {
    fn serialize(&self) -> Vec<u8> { self.clone() }
}

impl Deserialize for Vec<u8> {
    fn deserialize(bytes: &[u8]) -> Result<Self, Error> { Ok(bytes.to_vec()) }
}

impl Serialize for PsbtSighashType {
    fn serialize(&self) -> Vec<u8> { consensus::serialize(&self.to_u32()) }
}

impl Deserialize for PsbtSighashType {
    fn deserialize(bytes: &[u8]) -> Result<Self, Error> {
        let raw: u32 = consensus::deserialize(bytes)?;
        Ok(PsbtSighashType { inner: raw })
    }
}

// Taproot related ser/deser
impl Serialize for XOnlyPublicKey {
    fn serialize(&self) -> Vec<u8> { XOnlyPublicKey::serialize(self).to_vec() }
}

impl Deserialize for XOnlyPublicKey {
    fn deserialize(bytes: &[u8]) -> Result<Self, Error> {
        XOnlyPublicKey::from_slice(bytes).map_err(|_| Error::InvalidXOnlyPublicKey)
    }
}

impl Serialize for taproot::Signature {
    fn serialize(&self) -> Vec<u8> { self.to_vec() }
}

impl Deserialize for taproot::Signature {
    fn deserialize(bytes: &[u8]) -> Result<Self, Error> {
        use taproot::SigFromSliceError::*;

        taproot::Signature::from_slice(bytes).map_err(|e| match e {
            SighashType(err) => Error::NonStandardSighashType(err.0),
            InvalidSignatureSize(_) => Error::InvalidTaprootSignature(e),
            Secp256k1(..) => Error::InvalidTaprootSignature(e),
            _ => panic!("TODO: Handle non_exhaustive error"),
        })
    }
}

impl Serialize for (XOnlyPublicKey, TapLeafHash) {
    fn serialize(&self) -> Vec<u8> {
        let ser_pk = self.0.serialize();
        let mut buf = Vec::with_capacity(ser_pk.len() + self.1.as_byte_array().len());
        buf.extend(&ser_pk);
        buf.extend(self.1.as_byte_array());
        buf
    }
}

impl Deserialize for (XOnlyPublicKey, TapLeafHash) {
    fn deserialize(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() < 32 {
            return Err(Error::NotEnoughData);
        }
        let a: XOnlyPublicKey = Deserialize::deserialize(&bytes[..32])?;
        let b: TapLeafHash = Deserialize::deserialize(&bytes[32..])?;
        Ok((a, b))
    }
}

impl Serialize for ControlBlock {
    fn serialize(&self) -> Vec<u8> { ControlBlock::serialize(self) }
}

impl Deserialize for ControlBlock {
    fn deserialize(bytes: &[u8]) -> Result<Self, Error> {
        Self::decode(bytes).map_err(|_| Error::InvalidControlBlock)
    }
}

// Versioned ScriptBuf
impl Serialize for (ScriptBuf, LeafVersion) {
    fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(self.0.len() + 1);
        buf.extend(self.0.as_bytes());
        buf.push(self.1.to_consensus());
        buf
    }
}

impl Deserialize for (ScriptBuf, LeafVersion) {
    fn deserialize(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.is_empty() {
            return Err(Error::NotEnoughData);
        }
        // The last byte is LeafVersion.
        let script = ScriptBuf::deserialize(&bytes[..bytes.len() - 1])?;
        let leaf_ver = LeafVersion::from_consensus(bytes[bytes.len() - 1])
            .map_err(|_| Error::InvalidLeafVersion)?;
        Ok((script, leaf_ver))
    }
}

impl Serialize for (Vec<TapLeafHash>, KeySource) {
    fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(32 * self.0.len() + key_source_len(&self.1));
        self.0.consensus_encode(&mut buf).expect("Vecs don't error allocation");
        // TODO: Add support for writing into a writer for key-source
        buf.extend(self.1.serialize());
        buf
    }
}

impl Deserialize for (Vec<TapLeafHash>, KeySource) {
    fn deserialize(bytes: &[u8]) -> Result<Self, Error> {
        let (leafhash_vec, consumed) = consensus::deserialize_partial::<Vec<TapLeafHash>>(bytes)?;
        let key_source = KeySource::deserialize(&bytes[consumed..])?;
        Ok((leafhash_vec, key_source))
    }
}

impl Serialize for TapTree {
    fn serialize(&self) -> Vec<u8> {
        let capacity = self
            .script_leaves()
            .map(|l| {
                l.script().len() + VarInt::from(l.script().len()).size() // script version
            + 1 // merkle branch
            + 1 // leaf version
            })
            .sum::<usize>();
        let mut buf = Vec::with_capacity(capacity);
        for leaf_info in self.script_leaves() {
            // # Cast Safety:
            //
            // TaprootMerkleBranch can only have len atmost 128(TAPROOT_CONTROL_MAX_NODE_COUNT).
            // safe to cast from usize to u8
            buf.push(leaf_info.merkle_branch().len() as u8);
            buf.push(leaf_info.version().to_consensus());
            leaf_info.script().consensus_encode(&mut buf).expect("Vecs dont err");
        }
        buf
    }
}

impl Deserialize for TapTree {
    fn deserialize(bytes: &[u8]) -> Result<Self, Error> {
        let mut builder = TaprootBuilder::new();
        let mut bytes_iter = bytes.iter();
        while let Some(depth) = bytes_iter.next() {
            let version = bytes_iter.next().ok_or(Error::Taproot("Invalid Taproot Builder"))?;
            let (script, consumed) =
                consensus::deserialize_partial::<ScriptBuf>(bytes_iter.as_slice())?;
            if consumed > 0 {
                bytes_iter.nth(consumed - 1);
            }
            let leaf_version =
                LeafVersion::from_consensus(*version).map_err(|_| Error::InvalidLeafVersion)?;
            builder = builder
                .add_leaf_with_ver(*depth, script, leaf_version)
                .map_err(|_| Error::Taproot("Tree not in DFS order"))?;
        }
        TapTree::try_from(builder).map_err(Error::TapTree)
    }
}

// Helper function to compute key source len
fn key_source_len(key_source: &KeySource) -> usize { 4 + 4 * (key_source.1).as_ref().len() }

// TODO: This error is still too general but splitting it up is
// non-trivial because it is returned by the Deserialize trait.
/// Ways that deserializing a PSBT might fail.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// Not enough data to deserialize object.
    NotEnoughData,
    /// Non-proprietary key type found when proprietary key was expected
    InvalidProprietaryKey,
    /// Signals that there are no more key-value pairs in a key-value map.
    NoMorePairs,
    /// Unable to parse as a standard sighash type.
    NonStandardSighashType(u32),
    /// Invalid hash when parsing slice.
    InvalidHash(hashes::FromSliceError),
    /// Serialization error in bitcoin consensus-encoded structures
    ConsensusEncoding(consensus::encode::Error),
    /// Parsing error indicating invalid public keys
    InvalidPublicKey(bitcoin::key::Error),
    /// Parsing error indicating invalid secp256k1 public keys
    InvalidSecp256k1PublicKey(secp256k1::Error),
    /// Parsing error indicating invalid xonly public keys
    InvalidXOnlyPublicKey,
    /// Parsing error indicating invalid ECDSA signatures
    InvalidEcdsaSignature(bitcoin::ecdsa::Error),
    /// Parsing error indicating invalid taproot signatures
    InvalidTaprootSignature(bitcoin::taproot::SigFromSliceError),
    /// Parsing error indicating invalid control block
    InvalidControlBlock,
    /// Parsing error indicating invalid leaf version
    InvalidLeafVersion,
    /// Parsing error indicating a taproot error
    Taproot(&'static str),
    /// Taproot tree deserilaization error
    TapTree(taproot::IncompleteBuilderError),
    /// Error related to PSBT version
    /// PSBT data is not consumed entirely
    PartialDataConsumption,
    /// Couldn't converting parsed u32 to a lock time.
    LockTime(absolute::Error),
    /// Unsupported PSBT version.
    UnsupportedVersion(version::UnsupportedVersionError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use Error::*;

        match *self {
            NotEnoughData => f.write_str("not enough data to deserialize object"),
            InvalidProprietaryKey =>
                write!(f, "non-proprietary key type found when proprietary key was expected"),
            NoMorePairs => f.write_str("no more key-value pairs for this psbt map"),
            NonStandardSighashType(ref sht) => write!(f, "non-standard sighash type: {}", sht),
            InvalidHash(ref e) => write_err!(f, "invalid hash when parsing slice"; e),
            ConsensusEncoding(ref e) => write_err!(f, "bitcoin consensus encoding error"; e),
            InvalidPublicKey(ref e) => write_err!(f, "invalid public key"; e),
            InvalidSecp256k1PublicKey(ref e) => write_err!(f, "invalid secp256k1 public key"; e),
            InvalidXOnlyPublicKey => f.write_str("invalid xonly public key"),
            InvalidEcdsaSignature(ref e) => write_err!(f, "invalid ECDSA signature"; e),
            InvalidTaprootSignature(ref e) => write_err!(f, "invalid taproot signature"; e),
            InvalidControlBlock => f.write_str("invalid control block"),
            InvalidLeafVersion => f.write_str("invalid leaf version"),
            Taproot(s) => write!(f, "taproot error -  {}", s),
            TapTree(ref e) => write_err!(f, "taproot tree error"; e),
            PartialDataConsumption =>
                f.write_str("data not consumed entirely when explicitly deserializing"),
            LockTime(ref e) => write_err!(f, "parsed locktime invalid"; e),
            UnsupportedVersion(ref e) => write_err!(f, "unsupported version"; e),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        use Error::*;

        match *self {
            InvalidHash(ref e) => Some(e),
            ConsensusEncoding(ref e) => Some(e),
            LockTime(ref e) => Some(e),
            UnsupportedVersion(ref e) => Some(e),
            NotEnoughData
            | InvalidProprietaryKey
            | NoMorePairs
            | NonStandardSighashType(_)
            | InvalidPublicKey(_)
            | InvalidSecp256k1PublicKey(_)
            | InvalidXOnlyPublicKey
            | InvalidEcdsaSignature(_)
            | InvalidTaprootSignature(_)
            | InvalidControlBlock
            | InvalidLeafVersion
            | Taproot(_)
            | TapTree(_)
            | PartialDataConsumption => None,
        }
    }
}

impl From<hashes::FromSliceError> for Error {
    fn from(e: hashes::FromSliceError) -> Self { Self::InvalidHash(e) }
}

impl From<consensus::encode::Error> for Error {
    fn from(e: consensus::encode::Error) -> Self { Self::ConsensusEncoding(e) }
}

impl From<absolute::Error> for Error {
    fn from(e: absolute::Error) -> Self { Self::LockTime(e) }
}

impl From<version::UnsupportedVersionError> for Error {
    fn from(e: version::UnsupportedVersionError) -> Self { Self::UnsupportedVersion(e) }
}

#[cfg(test)]
mod tests {
    use core::convert::TryFrom;

    use super::*;

    // Composes tree matching a given depth map, filled with dumb script leafs,
    // each of which consists of a single push-int op code, with int value
    // increased for each consecutive leaf.
    pub fn compose_taproot_builder<'map>(
        opcode: u8,
        depth_map: impl IntoIterator<Item = &'map u8>,
    ) -> TaprootBuilder {
        let mut val = opcode;
        let mut builder = TaprootBuilder::new();
        for depth in depth_map {
            let script = ScriptBuf::from_hex(&format!("{:02x}", val)).unwrap();
            builder = builder.add_leaf(*depth, script).unwrap();
            let (new_val, _) = val.overflowing_add(1);
            val = new_val;
        }
        builder
    }

    #[test]
    fn taptree_hidden() {
        let mut builder = compose_taproot_builder(0x51, &[2, 2, 2]);
        builder = builder
            .add_leaf_with_ver(
                3,
                ScriptBuf::from_hex("b9").unwrap(),
                LeafVersion::from_consensus(0xC2).unwrap(),
            )
            .unwrap();
        builder = builder.add_hidden_node(3, TapNodeHash::all_zeros()).unwrap();
        assert!(TapTree::try_from(builder).is_err());
    }

    #[test]
    fn taptree_roundtrip() {
        let mut builder = compose_taproot_builder(0x51, &[2, 2, 2, 3]);
        builder = builder
            .add_leaf_with_ver(
                3,
                ScriptBuf::from_hex("b9").unwrap(),
                LeafVersion::from_consensus(0xC2).unwrap(),
            )
            .unwrap();
        let tree = TapTree::try_from(builder).unwrap();
        let tree_prime = TapTree::deserialize(&tree.serialize()).unwrap();
        assert_eq!(tree, tree_prime);
    }

    #[test]
    fn can_deserialize_non_standard_psbt_sighash_type() {
        let non_standard_sighash = [222u8, 0u8, 0u8, 0u8]; // 32 byte value.
        let sighash = PsbtSighashType::deserialize(&non_standard_sighash);
        assert!(sighash.is_ok())
    }

    #[test]
    #[should_panic(expected = "InvalidMagic")]
    fn invalid_vector_1() {
        let hex_psbt = b"0200000001268171371edff285e937adeea4b37b78000c0566cbb3ad64641713ca42171bf6000000006a473044022070b2245123e6bf474d60c5b50c043d4c691a5d2435f09a34a7662a9dc251790a022001329ca9dacf280bdf30740ec0390422422c81cb45839457aeb76fc12edd95b3012102657d118d3357b8e0f4c2cd46db7b39f6d9c38d9a70abcb9b2de5dc8dbfe4ce31feffffff02d3dff505000000001976a914d0c59903c5bac2868760e90fd521a4665aa7652088ac00e1f5050000000017a9143545e6e33b832c47050f24d3eeb93c9c03948bc787b32e1300";
        crate::v0::Psbt::deserialize(hex_psbt).unwrap();
    }
}
