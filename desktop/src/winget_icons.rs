use crate::winget::Package;

pub fn candidates(package: &Package) -> Vec<String> {
    let mut urls = Vec::new();
    let mut icon_fields = package.icon_ref.split(':');
    match (icon_fields.next(), icon_fields.next(), icon_fields.next()) {
        (Some("sh"), Some(name), None) if safe_slug(name) => {
            urls.push(format!(
                "https://cdn.jsdelivr.net/gh/selfhst/icons/png/{name}.png"
            ));
        }
        (Some("di"), Some(name), None) if safe_slug(name) => {
            urls.push(format!(
                "https://cdn.jsdelivr.net/gh/homarr-labs/dashboard-icons/png/{name}.png"
            ));
        }
        (Some("si"), Some(name), Some(color)) if safe_slug(name) && safe_color(color) => {
            urls.push(format!("https://cdn.simpleicons.org/{name}/{color}.svg"));
        }
        _ => {}
    }
    if let Some(owner) = package
        .domain
        .strip_prefix("github.com/")
        .and_then(|path| path.split('/').next())
        .filter(|owner| safe_slug(owner))
    {
        urls.push(format!("https://github.com/{owner}.png?size=128"));
    }
    if !safe_domain(&package.domain) {
        return urls;
    }
    let page = if package.website.starts_with("https://") || package.website.starts_with("http://")
    {
        package.website.as_str()
    } else {
        ""
    };
    let page = if page.is_empty() {
        format!("https://{}", package.domain)
    } else {
        page.to_owned()
    };
    let encoded_page = percent_encode(&page);
    urls.push(format!(
        "https://www.google.com/s2/favicons?sz=128&domain_url={encoded_page}"
    ));
    urls.push(format!("https://t1.gstatic.com/faviconV2?client=SOCIAL&type=FAVICON&fallback_opts=TYPE,SIZE,URL&url={encoded_page}&size=128"));
    let host = package.domain.split('/').next().unwrap_or_default();
    urls.push(format!("https://icons.duckduckgo.com/ip3/{host}.ico"));
    urls
}

fn safe_slug(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn safe_color(value: &str) -> bool {
    value.len() == 6 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn safe_domain(value: &str) -> bool {
    let Some((host, path)) = value.split_once('/') else {
        return valid_host(value);
    };
    host == "github.com" && !path.is_empty() && path.split('/').all(safe_slug)
}

fn valid_host(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
        && !value.starts_with('.')
        && !value.ends_with('.')
        && !value.contains("..")
}

fn percent_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push('%');
            encoded.push_str(&format!("{byte:02X}"));
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(domain: &str, website: &str, icon_ref: &str) -> Package {
        Package {
            category: "Utilities".into(),
            name: "Sample App".into(),
            id: "Sample.App".into(),
            domain: domain.into(),
            website: website.into(),
            icon_ref: icon_ref.into(),
        }
    }

    #[test]
    fn uses_curated_icon_pack_then_product_site_fallbacks() {
        let urls = candidates(&package("brave.com", "https://www.brave.com/", "sh:brave"));
        assert_eq!(
            urls[0],
            "https://cdn.jsdelivr.net/gh/selfhst/icons/png/brave.png"
        );
        assert!(urls[1].contains("brave.com"));
        assert_eq!(urls.len(), 4);

        let dashboard_icon = candidates(&package("vivaldi.com", "", "di:vivaldi"));
        assert_eq!(
            dashboard_icon[0],
            "https://cdn.jsdelivr.net/gh/homarr-labs/dashboard-icons/png/vivaldi.png"
        );
    }

    #[test]
    fn github_entries_use_the_organization_avatar_and_ignore_unsafe_refs() {
        let urls = candidates(&package("github.com/Hibbiki", "", "sh:../../other"));
        assert_eq!(urls[0], "https://github.com/Hibbiki.png?size=128");
        assert_eq!(urls.len(), 4);
    }

    #[test]
    fn uses_colored_simple_icons_for_valid_slugs_and_colors() {
        let urls = candidates(&package("cmake.org", "", "si:cmake:064F8C"));
        assert_eq!(urls[0], "https://cdn.simpleicons.org/cmake/064F8C.svg");
        assert_eq!(urls.len(), 4);

        let unsafe_icon = candidates(&package("cmake.org", "", "si:../cmake:064F8C"));
        assert!(!unsafe_icon[0].contains("../"));
        let invalid_color = candidates(&package("cmake.org", "", "si:cmake:bad"));
        assert!(!invalid_color[0].contains("cdn.simpleicons.org"));
    }

    #[test]
    fn unsafe_domains_do_not_become_remote_urls() {
        assert!(candidates(&package("example.com/?token=secret", "", "")).is_empty());
    }

    #[test]
    fn every_catalog_package_has_at_least_one_icon_source() {
        let packages = crate::winget::catalog().unwrap();
        let missing = packages
            .iter()
            .filter(|package| candidates(package).is_empty())
            .map(|package| package.id.as_str())
            .collect::<Vec<_>>();
        assert!(
            missing.is_empty(),
            "packages without icon sources: {missing:?}"
        );
    }

    #[test]
    fn firefox_esr_uses_the_firefox_brand_icon() {
        let package = crate::winget::catalog()
            .unwrap()
            .into_iter()
            .find(|package| package.id == "Mozilla.Firefox.ESR")
            .expect("Firefox ESR must remain in the WinGet catalog");
        assert_eq!(
            candidates(&package)[0],
            "https://cdn.jsdelivr.net/gh/selfhst/icons/png/firefox.png"
        );
    }
}
