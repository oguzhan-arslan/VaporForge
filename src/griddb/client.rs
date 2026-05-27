use serde::Deserialize;
use steamgriddb_api::images::get_images_by_game_id_url;
use steamgriddb_api::query_parameters::{
    AnimtionType, GridDimentions, GridQueryParameters, Humor, Nsfw, QueryType,
};

// Re-export crate types so callers don't need to depend on steamgriddb_api directly.
pub use steamgriddb_api::images::Image as ImageResult;
pub use steamgriddb_api::search::SearchResult as GameResult;

// Static dimension lists for portrait vs landscape grid queries.
static COVER_DIMS: [GridDimentions; 3] = [
    GridDimentions::D600x900,
    GridDimentions::D342x482,
    GridDimentions::D660x930,
];
static WIDE_COVER_DIMS: [GridDimentions; 2] = [
    GridDimentions::D460x215,
    GridDimentions::D920x430,
];

static STATIC_ONLY: [AnimtionType; 1] = [AnimtionType::Static];

/// The artwork kinds Steam displays for non-Steam shortcuts, using Steam's own names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageKind {
    /// Vertical portrait card shown in the Steam library grid — stored as `{appid}p.png`
    Cover,
    /// Horizontal capsule shown on hover and for the last-played game — stored as `{appid}.png`
    WideCover,
    /// Full-width banner shown on the game detail page — stored as `{appid}_hero.png`
    Background,
    /// Transparent game-name logo overlaid on the detail page — stored as `{appid}_logo.png`
    Logo,
    /// Small icon — stored as `{appid}_icon.png`
    Icon,
}

impl ImageKind {
    pub(crate) fn to_query_type<'a>(&self, opts: &'a GetImagesOptions) -> QueryType<'a> {
        let anim_types: &'a [AnimtionType] = &STATIC_ONLY;
        let nsfw = if opts.show_nsfw { &Nsfw::Any } else { &Nsfw::False };
        let humor = if opts.show_humor { &Humor::Any } else { &Humor::False };

        match self {
            ImageKind::Cover => QueryType::Grid(Some(GridQueryParameters {
                dimentions: Some(&COVER_DIMS),
                types: Some(anim_types),
                nsfw: Some(nsfw),
                humor: Some(humor),
                ..Default::default()
            })),
            ImageKind::WideCover => QueryType::Grid(Some(GridQueryParameters {
                dimentions: Some(&WIDE_COVER_DIMS),
                types: Some(anim_types),
                nsfw: Some(nsfw),
                humor: Some(humor),
                ..Default::default()
            })),
            ImageKind::Background => QueryType::Hero(Some(
                steamgriddb_api::query_parameters::HeroQueryParameters {
                    types: Some(anim_types),
                    nsfw: Some(nsfw),
                    humor: Some(humor),
                    ..Default::default()
                },
            )),
            ImageKind::Logo => QueryType::Logo(Some(
                steamgriddb_api::query_parameters::LogoQueryParameters {
                    types: Some(anim_types),
                    nsfw: Some(nsfw),
                    humor: Some(humor),
                    ..Default::default()
                },
            )),
            ImageKind::Icon => QueryType::Icon(Some(
                steamgriddb_api::query_parameters::IconQueryParameters {
                    types: Some(anim_types),
                    nsfw: Some(nsfw),
                    humor: Some(humor),
                    ..Default::default()
                },
            )),
        }
    }

    /// Filename suffix appended after the appid (before the extension).
    pub fn filename_suffix(&self) -> &'static str {
        match self {
            ImageKind::Cover => "p",
            ImageKind::WideCover => "",
            ImageKind::Background => "_hero",
            ImageKind::Logo => "_logo",
            ImageKind::Icon => "_icon",
        }
    }
}

/// Options forwarded to the SteamGridDB API for an image query.
#[derive(Debug, Clone)]
pub struct GetImagesOptions {
    /// Zero-based page index.
    pub page: usize,
    /// Images per page (10–50).
    pub limit: usize,
    pub show_nsfw: bool,
    pub show_humor: bool,
}

impl Default for GetImagesOptions {
    fn default() -> Self {
        Self {
            page: 0,
            limit: 25,
            show_nsfw: false,
            show_humor: false,
        }
    }
}

#[derive(Deserialize)]
struct ApiResponse {
    success: Option<bool>,
    data: Option<Vec<ImageResult>>,
    errors: Option<Vec<String>>,
}

pub struct GridDbClient {
    inner: steamgriddb_api::Client,
    http: reqwest::Client,
}

impl GridDbClient {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            inner: steamgriddb_api::Client::new(api_key),
            http: reqwest::Client::new(),
        }
    }

    /// Search SteamGridDB for games matching `name`.
    pub async fn search_game(&self, name: &str) -> eyre::Result<Vec<GameResult>> {
        self.inner
            .search(name)
            .await
            .map_err(|e| eyre::eyre!("{e}"))
    }

    /// Fetch one page of images for a SteamGridDB game ID.
    pub async fn get_images(
        &self,
        game_id: usize,
        kind: ImageKind,
        opts: &GetImagesOptions,
    ) -> eyre::Result<Vec<ImageResult>> {
        let query = kind.to_query_type(opts);
        let base = get_images_by_game_id_url(self.inner.base_url(), game_id, &query);
        let url = format!(
            "{}{}&limit={}&page={}",
            base,
            if base.contains('?') { "" } else { "?" },
            opts.limit,
            opts.page,
        );

        let resp = self.http
            .get(&url)
            .bearer_auth(self.inner.get_auth_key())
            .send()
            .await
            .map_err(eyre::Report::from)?
            .json::<ApiResponse>()
            .await
            .map_err(eyre::Report::from)?;

        if resp.success == Some(false) {
            let msg = resp.errors
                .filter(|e| !e.is_empty())
                .map(|e| e.join(", "))
                .unwrap_or_else(|| "API request failed".to_string());
            return Err(eyre::eyre!("{msg}"));
        }

        Ok(resp.data.unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use steamgriddb_api::query_parameters::QueryType;

    fn default_opts() -> GetImagesOptions {
        GetImagesOptions::default()
    }

    #[test]
    fn image_kind_query_mapping() {
        let opts = default_opts();
        assert!(matches!(ImageKind::Cover.to_query_type(&opts), QueryType::Grid(_)));
        assert!(matches!(ImageKind::WideCover.to_query_type(&opts), QueryType::Grid(_)));
        assert!(matches!(ImageKind::Background.to_query_type(&opts), QueryType::Hero(_)));
        assert!(matches!(ImageKind::Logo.to_query_type(&opts), QueryType::Logo(_)));
        assert!(matches!(ImageKind::Icon.to_query_type(&opts), QueryType::Icon(_)));
    }

    #[test]
    fn cover_uses_portrait_dimensions() {
        let opts = default_opts();
        if let QueryType::Grid(Some(params)) = ImageKind::Cover.to_query_type(&opts) {
            let dims = params.dimentions.expect("Cover must have dimension filter");
            assert!(dims.iter().all(|d| matches!(
                d,
                GridDimentions::D600x900 | GridDimentions::D342x482 | GridDimentions::D660x930
            )));
        } else {
            panic!("Cover should be Grid with dimension params");
        }
    }

    #[test]
    fn wide_cover_uses_landscape_dimensions() {
        let opts = default_opts();
        if let QueryType::Grid(Some(params)) = ImageKind::WideCover.to_query_type(&opts) {
            let dims = params.dimentions.expect("WideCover must have dimension filter");
            assert!(dims
                .iter()
                .all(|d| matches!(d, GridDimentions::D460x215 | GridDimentions::D920x430)));
        } else {
            panic!("WideCover should be Grid with dimension params");
        }
    }

    #[test]
    fn default_opts_exclude_animated() {
        let opts = default_opts();
        if let QueryType::Grid(Some(params)) = ImageKind::Cover.to_query_type(&opts) {
            let types = params.types.expect("should have types filter");
            assert_eq!(types, &[AnimtionType::Static]);
        } else {
            panic!("Cover should be Grid");
        }
    }

    #[test]
    fn show_animated_includes_animated_type() {
        let opts = GetImagesOptions { show_animated: true, ..default_opts() };
        if let QueryType::Grid(Some(params)) = ImageKind::Cover.to_query_type(&opts) {
            let types = params.types.expect("should have types filter");
            assert!(types.contains(&AnimtionType::Animated));
            assert!(types.contains(&AnimtionType::Static));
        } else {
            panic!("Cover should be Grid");
        }
    }

    #[test]
    fn filename_suffixes_are_correct() {
        assert_eq!(ImageKind::Cover.filename_suffix(), "p");
        assert_eq!(ImageKind::WideCover.filename_suffix(), "");
        assert_eq!(ImageKind::Background.filename_suffix(), "_hero");
        assert_eq!(ImageKind::Logo.filename_suffix(), "_logo");
        assert_eq!(ImageKind::Icon.filename_suffix(), "_icon");
    }

    #[test]
    fn client_stores_api_key() {
        let client = GridDbClient::new("test-key");
        assert_eq!(client.inner.get_auth_key(), "test-key");
    }
}
