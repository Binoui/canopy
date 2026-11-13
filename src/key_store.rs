use secret_service::{blocking::SecretService, EncryptionType};
use std::collections::HashMap;
use zeroize::Zeroizing;

pub(crate) fn read(attributes: &[(&str, &str)]) -> Result<Option<Zeroizing<Vec<u8>>>, ()> {
    let service = SecretService::connect(EncryptionType::Dh).map_err(|_| ())?;
    let found = service
        .search_items(attributes.iter().copied().collect())
        .map_err(|_| ())?;
    if let Some(item) = found.unlocked.into_iter().chain(found.locked).next() {
        item.unlock().map_err(|_| ())?;
        return item
            .get_secret()
            .map(Zeroizing::new)
            .map(Some)
            .map_err(|_| ());
    }
    Ok(None)
}

pub(crate) fn write(attributes: &[(&str, &str)], key: &[u8]) -> Result<(), ()> {
    let service = SecretService::connect(EncryptionType::Dh).map_err(|_| ())?;
    let collection = service.get_default_collection().map_err(|_| ())?;
    collection.unlock().map_err(|_| ())?;
    let attributes: HashMap<_, _> = attributes.iter().copied().collect();
    collection
        .create_item(
            "Canopy Safe Storage",
            attributes,
            key,
            true,
            "application/octet-stream",
        )
        .map(|_| ())
        .map_err(|_| ())
}
