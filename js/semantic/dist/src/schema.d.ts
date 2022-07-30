export declare const FACTOR_ID = "factor/id";
export declare const FACTOR_IDENT = "factor/ident";
export declare const FACTOR_TITLE = "factor/title";
export declare const FACTOR_DESCRIPTION = "factor/description";
export declare const FACTOR_TYPE = "factor/type";
export declare const FACTOR_VALUE_TYPE = "factor/valueType";
export declare const FACTOR_UNIQUE = "factor/unique";
export declare const FACTOR_INDEX = "factor/index";
export declare const FACTOR_IS_STRICT = "factor/isStrict";
export declare const FACTOR_ENTITY_ATTRIBUTES = "factor/entityAttributes";
export declare const FACTOR_EXTEND = "factor/extend";
export declare const FACTOR_IS_RELATION = "factor/isRelation";
export declare const FACTOR_INDEX_ATTRIBUTES = "factor/index_attributes";
export declare const FACTOR_COUNT = "factor/count";
export declare const SEMANTIC_TITLE = "semantic/title";
export declare const SEMANTIC_COMMENT = "semantic/comment";
export declare const SEMANTIC_DESCRIPTION = "semantic/description";
export declare const SEMANTIC_URL = "semantic/url";
export declare const SEMANTIC_PREVIEW_IMAGE_URL = "semantic/preview_image_url";
export declare const SEMANTIC_PREVIEW_IMAGE_BLOB_URI = "semantic/preview_image_blob_uri";
export declare const SEMANTIC_USERNAME = "semantic/username";
export declare const SEMANTIC_TEXT_FORMAT = "semantic/text_format";
export declare const SEMANTIC_CREATED_AT = "semantic/created_at";
export declare const SEMANTIC_UPDATED_AT = "semantic/updated_at";
export declare const SEMANTIC_VISIT_COUNT = "semantic/visit_count";
export declare const SEMANTIC_LAST_VISIT_TIME = "semantic/last_visit_time";
export declare const SEMANTIC_NAME = "semantic/name";
export declare const SEMANTIC_SECONDARY_URL = "semantic/secondary_url";
export declare const SEMANTIC_PARENT = "semantic/parent";
export declare const SEMANTIC_EMBEDDED_IN_PARENT = "semantic/embedded_in_parent";
export declare const SEMANTIC_BLOB_URI = "semantic/blob_uri";
export declare const SEMANTIC_BLOB_URI_WEB = "semantic/blob_uri_web";
export declare const SEMANTIC_MIME_TYPE = "semantic/mime_type";
export declare const SEMANTIC_HASH = "semantic/hash";
export declare const SEMANTIC_ORIGINAL_HASH = "semantic/original_hash";
export declare const SEMANTIC_DURATION = "semantic/duration";
export declare const SEMANTIC_FILE_SIZE = "semantic/file_size";
export declare const SEMANTIC_DOWNLOAD_URL = "semantic/download_url";
export declare const SEMANTIC_FILENAME = "semantic/filename";
export declare const SEMANTIC_VIDEO_HAS_SOUND = "semantic/video_has_sound";
export declare const SEMANTIC_SOCIAL_MEDIA_POST_CONTENT = "semantic/social_media_post_content";
export declare const SEMANTIC_SOCIAL_MEDIA_POST_USER_ID = "semantic/social_media_post_user_id";
export declare const SEMANTIC_SOCIAL_MEDIA_PLATFORM_NAME = "semantic/social_media_platform_name";
export declare const SEMANTIC_SOCIAL_MEDIA_PLATFORM_ID = "semantic/social_media_platform_id";
export declare const SEMANTIC_NOTE_BODY = "semantic/note_body";
export declare const SEMANTIC_COLLECTION_ITEMS = "semantic/collection_items";
export declare const SEMANTIC_TAG_NAME = "semantic/tag_name";
export declare const SEMANTIC_TAG_PARENT = "semantic/tag_parent";
export declare const SEMANTIC_TAGS = "semantic/tags";
export declare const SEMANTIC_GIVEN_NAME = "semantic/given_name";
export declare const SEMANTIC_FAMILY_NAME = "semantic/family_name";
export declare const SEMANTIC_BIRTHDATE = "semantic/birthdate";
export declare const SEMANTIC_GENDER = "semantic/gender";
export declare const SEMANTIC_LIKE_COUNT = "semantic/like_count";
export declare type EntityId = string;
export declare type EntityIdent = string;
export declare type Ident = string;
export declare type IdOrIdent = EntityId | string;
export declare type Url = string;
export declare type Timestamp = number;
export interface BaseEntity {
    "factor/id": EntityId;
    "factor/ident"?: Ident | null;
    "factor/type"?: string | null;
}
export declare const TY_FACTOR_ATTRIBUTE = "factor/Attribute";
export interface FactorAttribute extends BaseEntity {
    "factor/type": "factor/Attribute";
    "factor/id": EntityId;
    "factor/ident": string;
    "factor/title"?: string | null;
    "factor/description"?: string | null;
    "factor/valueType": string;
    "factor/unique": boolean;
    "factor/index": boolean;
    "factor/isStrict": boolean;
}
export declare const TY_FACTOR_ENTITY = "factor/Entity";
export interface FactorEntity extends BaseEntity {
    "factor/type": "factor/Entity";
    "factor/id": EntityId;
    "factor/ident": string;
    "factor/title"?: string | null;
    "factor/description"?: string | null;
    "factor/isStrict": boolean;
    "factor/isRelation": boolean;
    "factor/extend": EntityIdent[];
    "factor/entityAttributes": {
        attribute: EntityIdent;
        cardinality: "Optional" | "Required";
    }[];
}
export declare const TY_FACTOR_INDEX = "factor/Index";
export interface FactorIndex extends BaseEntity {
    "factor/type": "factor/Index";
    "factor/id": EntityId;
    "factor/ident": string;
    "factor/title"?: string | null;
    "factor/description"?: string | null;
    "factor/index_attributes": EntityId[];
}
export declare const TY_SEMANTIC_FILE = "semantic/File";
export interface SemanticFile extends BaseEntity {
    "factor/type": "semantic/File";
    "factor/ident"?: string | null;
    "semantic/title"?: string | null;
    "semantic/filename"?: string | null;
    "semantic/file_size"?: number | null;
    "semantic/mime_type"?: string | null;
    "semantic/hash"?: string | null;
    "semantic/original_hash"?: string | null;
    "semantic/url"?: Url | null;
    "semantic/download_url"?: Url | null;
    "semantic/preview_image_url"?: Url | null;
    "semantic/preview_image_blob_uri"?: string | null;
    "semantic/blob_uri"?: string | null;
    "semantic/blob_uri_web"?: string | null;
    "semantic/created_at"?: Timestamp | null;
    "semantic/updated_at"?: Timestamp | null;
}
export declare const TY_SEMANTIC_IMAGE = "semantic/Image";
export interface SemanticImage extends Omit<SemanticFile, "factor/type"> {
    "factor/type": "semantic/Image";
}
export declare const TY_SEMANTIC_VIDEO = "semantic/Video";
export interface SemanticVideo extends Omit<SemanticFile, "factor/type"> {
    "factor/type": "semantic/Video";
    "semantic/duration"?: number | null;
    "semantic/video_has_sound"?: boolean | null;
}
export declare const TY_SEMANTIC_SOCIAL_MEDIA_POST = "semantic/SocialMediaPost";
export interface SemanticSocialMediaPost extends BaseEntity {
    "factor/type": "semantic/SocialMediaPost";
    "factor/ident"?: string | null;
    "semantic/title"?: string | null;
    "semantic/username"?: string | null;
    "semantic/social_media_post_user_id"?: EntityId | null;
    "semantic/like_count"?: number | null;
    "semantic/social_media_post_content": EntityId[];
}
export declare const TY_SEMANTIC_SOCIAL_MEDIA_ACCOUNT = "semantic/SocialMediaAccount";
export interface SemanticSocialMediaAccount extends Omit<SemanticPerson, "factor/type"> {
    "factor/type": "semantic/SocialMediaAccount";
    "semantic/social_media_platform_name"?: string | null;
    "semantic/social_media_platform_id"?: EntityId | null;
    "semantic/username": string;
}
export declare const TY_SEMANTIC_NOTE = "semantic/Note";
export interface SemanticNote extends BaseEntity {
    "factor/type": "semantic/Note";
    "semantic/title": string;
    "semantic/note_body": string;
    "semantic/text_format": "plain" | "markdown";
}
export declare const TY_SEMANTIC_COLLECTION = "semantic/Collection";
export interface SemanticCollection extends BaseEntity {
    "factor/type": "semantic/Collection";
    "factor/ident"?: string | null;
    "semantic/url"?: Url | null;
    "semantic/title": string;
    "semantic/description"?: string | null;
    "semantic/collection_items": EntityId[];
}
export declare const TY_SEMANTIC_TAG = "semantic/Tag";
export interface SemanticTag extends BaseEntity {
    "factor/type": "semantic/Tag";
    "semantic/tag_name": string;
    "semantic/description"?: string | null;
    "semantic/tag_parent"?: EntityId | null;
}
export declare const TY_SEMANTIC_PERSON = "semantic/Person";
export interface SemanticPerson extends BaseEntity {
    "factor/type": "semantic/Person";
    "factor/ident"?: string | null;
    "semantic/description"?: string | null;
    "semantic/url"?: Url | null;
    "semantic/name"?: string | null;
    "semantic/given_name"?: string | null;
    "semantic/family_name"?: EntityId | null;
    "semantic/birthdate"?: EntityId | null;
    "semantic/gender"?: "male" | "female" | null;
}
