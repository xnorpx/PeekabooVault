//! MP4 Muxer - Creates proper ISO Base Media File Format (MP4) files
//!
//! This module handles parsing H.264 and H.265/HEVC NAL units and creating valid MP4 files
//! with proper sample tables for seeking and playback.
//!
//! Supported Codecs:
//! - H.264/AVC: Uses avc1 sample entry with avcC configuration box
//! - H.265/HEVC: Uses hvc1 sample entry with hvcC configuration box
//!
//! MP4 Box Structure:
//! ```text
//! ftyp                    - File type (isom brand)
//! moov                    - Movie container
//!   mvhd                  - Movie header (timescale, duration)
//!   trak                  - Track container
//!     tkhd                - Track header (dimensions)
//!     mdia                - Media container
//!       mdhd              - Media header (timescale, duration)
//!       hdlr              - Handler (vide = video)
//!       minf              - Media information
//!         vmhd            - Video media header
//!         dinf            - Data information
//!           dref          - Data reference
//!         stbl            - Sample table
//!           stsd          - Sample descriptions (avcC/hvcC with SPS/PPS/VPS)
//!           stts          - Time-to-sample (frame durations)
//!           stss          - Sync samples (keyframe indices)
//!           stsc          - Sample-to-chunk mapping
//!           stsz          - Sample sizes
//!           stco/co64     - Chunk offsets
//! mdat                    - Media data (actual frames)
//! ```

use bytes::{BufMut, Bytes, BytesMut};
use h264_reader::nal::sps::SeqParameterSet;
use h264_reader::rbsp::BitReader;
use tracing::{debug, warn};

/// Video codec type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VideoCodec {
    #[default]
    H264,
    H265,
}

/// H.264 NAL unit types (5-bit type field)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum H264NalType {
    /// Non-IDR slice
    Slice = 1,
    /// IDR slice (keyframe)
    Idr = 5,
    /// Supplemental Enhancement Information
    Sei = 6,
    /// Sequence Parameter Set
    Sps = 7,
    /// Picture Parameter Set
    Pps = 8,
    /// Access Unit Delimiter
    Aud = 9,
    /// Other/Unknown
    Other = 0,
}

impl H264NalType {
    fn from_header(header: u8) -> Self {
        match header & 0x1F {
            1 => H264NalType::Slice,
            5 => H264NalType::Idr,
            6 => H264NalType::Sei,
            7 => H264NalType::Sps,
            8 => H264NalType::Pps,
            9 => H264NalType::Aud,
            _ => H264NalType::Other,
        }
    }
    
    fn is_keyframe_nal(&self) -> bool {
        matches!(self, H264NalType::Idr)
    }
    
    fn is_parameter_set(&self) -> bool {
        matches!(self, H264NalType::Sps | H264NalType::Pps)
    }
}

/// H.265/HEVC NAL unit types (6-bit type field in bits 1-6 of first byte)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum H265NalType {
    /// Coded slice of a trailing picture (non-keyframe)
    TrailN = 0,
    TrailR = 1,
    /// Coded slice of TSA picture
    TsaN = 2,
    TsaR = 3,
    /// Coded slice of STSA picture
    StsaN = 4,
    StsaR = 5,
    /// Coded slice of RADL picture
    RadlN = 6,
    RadlR = 7,
    /// Coded slice of RASL picture
    RaslN = 8,
    RaslR = 9,
    /// Coded slice of BLA picture (keyframe)
    BlaWLp = 16,
    BlaWRadl = 17,
    BlaNLp = 18,
    /// Coded slice of IDR picture (keyframe)
    IdrWRadl = 19,
    IdrNLp = 20,
    /// Coded slice of CRA picture (keyframe)
    CraNut = 21,
    /// Video Parameter Set
    VpsNut = 32,
    /// Sequence Parameter Set
    SpsNut = 33,
    /// Picture Parameter Set
    PpsNut = 34,
    /// Access Unit Delimiter
    AudNut = 35,
    /// End of sequence
    EosNut = 36,
    /// End of bitstream
    EobNut = 37,
    /// Filler data
    FdNut = 38,
    /// Prefix SEI
    PrefixSeiNut = 39,
    /// Suffix SEI
    SuffixSeiNut = 40,
    /// Other/Unknown
    Other = 255,
}

impl H265NalType {
    fn from_header(header: u8) -> Self {
        // H.265 NAL header: forbidden_zero_bit(1) + nal_unit_type(6) + nuh_layer_id(6) + nuh_temporal_id_plus1(3)
        // First byte contains: forbidden(1) + type(6) + layer_id high bit(1)
        let nal_type = (header >> 1) & 0x3F;
        match nal_type {
            0 => H265NalType::TrailN,
            1 => H265NalType::TrailR,
            2 => H265NalType::TsaN,
            3 => H265NalType::TsaR,
            4 => H265NalType::StsaN,
            5 => H265NalType::StsaR,
            6 => H265NalType::RadlN,
            7 => H265NalType::RadlR,
            8 => H265NalType::RaslN,
            9 => H265NalType::RaslR,
            16 => H265NalType::BlaWLp,
            17 => H265NalType::BlaWRadl,
            18 => H265NalType::BlaNLp,
            19 => H265NalType::IdrWRadl,
            20 => H265NalType::IdrNLp,
            21 => H265NalType::CraNut,
            32 => H265NalType::VpsNut,
            33 => H265NalType::SpsNut,
            34 => H265NalType::PpsNut,
            35 => H265NalType::AudNut,
            36 => H265NalType::EosNut,
            37 => H265NalType::EobNut,
            38 => H265NalType::FdNut,
            39 => H265NalType::PrefixSeiNut,
            40 => H265NalType::SuffixSeiNut,
            _ => H265NalType::Other,
        }
    }
    
    /// Check if this is an IRAP (Intra Random Access Point) - keyframe types
    fn is_keyframe_nal(&self) -> bool {
        matches!(
            self,
            H265NalType::BlaWLp
                | H265NalType::BlaWRadl
                | H265NalType::BlaNLp
                | H265NalType::IdrWRadl
                | H265NalType::IdrNLp
                | H265NalType::CraNut
        )
    }
    
    fn is_parameter_set(&self) -> bool {
        matches!(self, H265NalType::VpsNut | H265NalType::SpsNut | H265NalType::PpsNut)
    }
}

/// Generic NAL unit type that works for both codecs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NalUnitType {
    H264(H264NalType),
    H265(H265NalType),
}

impl NalUnitType {
    pub fn is_keyframe(&self) -> bool {
        match self {
            NalUnitType::H264(t) => t.is_keyframe_nal(),
            NalUnitType::H265(t) => t.is_keyframe_nal(),
        }
    }
    
    pub fn is_parameter_set(&self) -> bool {
        match self {
            NalUnitType::H264(t) => t.is_parameter_set(),
            NalUnitType::H265(t) => t.is_parameter_set(),
        }
    }
}

// Keep old type alias for backward compatibility
pub type H264NalUnitType = H264NalType;

/// Parsed NAL unit
#[derive(Debug, Clone)]
pub struct NalUnit {
    pub nal_type: NalUnitType,
    pub data: Bytes,
}

/// Parsed video frame with NAL units (works for both H.264 and H.265)
#[derive(Debug, Clone)]
pub struct H264Frame {
    pub is_keyframe: bool,
    pub timestamp: u32,
    pub nal_units: Vec<NalUnit>,
    /// Total size of all NAL units (with length prefixes)
    pub total_size: usize,
}

/// Codec configuration for H.264
#[derive(Debug, Clone, Default)]
pub struct H264Config {
    pub sps: Vec<Bytes>,
    pub pps: Vec<Bytes>,
    pub profile_idc: u8,
    pub profile_compat: u8,
    pub level_idc: u8,
    pub width: u32,
    pub height: u32,
}

/// Codec configuration for H.265/HEVC
#[derive(Debug, Clone, Default)]
pub struct H265Config {
    pub vps: Vec<Bytes>,
    pub sps: Vec<Bytes>,
    pub pps: Vec<Bytes>,
    /// General profile space (2 bits)
    pub general_profile_space: u8,
    /// General tier flag
    pub general_tier_flag: bool,
    /// General profile IDC (5 bits)
    pub general_profile_idc: u8,
    /// General profile compatibility flags (32 bits)
    pub general_profile_compatibility_flags: u32,
    /// General constraint indicator flags (48 bits)
    pub general_constraint_indicator_flags: u64,
    /// General level IDC
    pub general_level_idc: u8,
    pub width: u32,
    pub height: u32,
    pub chroma_format_idc: u8,
    pub bit_depth_luma_minus8: u8,
    pub bit_depth_chroma_minus8: u8,
}

/// Sample entry for the sample table
#[derive(Debug, Clone)]
pub struct SampleEntry {
    pub size: u32,
    pub duration: u32,
    pub is_sync: bool,
    pub offset: u64,
}

/// Codec-specific configuration
#[derive(Debug, Clone)]
pub enum CodecConfig {
    H264(H264Config),
    H265(H265Config),
}

impl Default for CodecConfig {
    fn default() -> Self {
        CodecConfig::H264(H264Config::default())
    }
}

impl CodecConfig {
    pub fn width(&self) -> u32 {
        match self {
            CodecConfig::H264(c) => c.width,
            CodecConfig::H265(c) => c.width,
        }
    }
    
    pub fn height(&self) -> u32 {
        match self {
            CodecConfig::H264(c) => c.height,
            CodecConfig::H265(c) => c.height,
        }
    }
    
    pub fn codec(&self) -> VideoCodec {
        match self {
            CodecConfig::H264(_) => VideoCodec::H264,
            CodecConfig::H265(_) => VideoCodec::H265,
        }
    }
}

/// MP4 Muxer for creating valid MP4 files (H.264 and H.265)
pub struct Mp4Muxer {
    codec: VideoCodec,
    h264_config: H264Config,
    h265_config: H265Config,
    samples: Vec<SampleEntry>,
    mdat_data: BytesMut,
    timescale: u32,
    current_offset: u64,
}

impl Mp4Muxer {
    /// Create a new MP4 muxer
    pub fn new() -> Self {
        Self {
            codec: VideoCodec::H264,
            h264_config: H264Config::default(),
            h265_config: H265Config::default(),
            samples: Vec::new(),
            mdat_data: BytesMut::new(),
            timescale: 90000, // 90kHz (standard for H.264/H.265)
            current_offset: 0,
        }
    }
    
    /// Create a new MP4 muxer for a specific codec
    pub fn with_codec(codec: VideoCodec) -> Self {
        let mut muxer = Self::new();
        muxer.codec = codec;
        muxer
    }
    
    /// Detect codec from NAL unit data
    fn detect_codec(data: &[u8]) -> VideoCodec {
        if data.len() < 5 {
            return VideoCodec::H264; // Default
        }
        
        // Parse first NAL unit to determine codec
        // Skip start code if present
        let nal_start = if data[0..4] == [0, 0, 0, 1] {
            4
        } else if data[0..3] == [0, 0, 1] {
            3
        } else if data.len() > 4 {
            // AVCC format - skip length prefix
            4
        } else {
            0
        };
        
        if nal_start >= data.len() {
            return VideoCodec::H264;
        }
        
        let first_byte = data[nal_start];
        
        // H.264: forbidden_zero_bit(1) + nal_ref_idc(2) + nal_unit_type(5)
        // H.265: forbidden_zero_bit(1) + nal_unit_type(6) + nuh_layer_id[high bit](1)
        // 
        // Key difference: H.264 SPS type is 7 (0x67 with nal_ref_idc=3)
        //                 H.265 VPS type is 32, SPS type is 33
        
        let h264_type = first_byte & 0x1F;
        let h265_type = (first_byte >> 1) & 0x3F;
        
        // H.265 parameter sets are in range 32-34
        if h265_type >= 32 && h265_type <= 34 {
            return VideoCodec::H265;
        }
        
        // H.264 parameter sets are 7 (SPS) and 8 (PPS)
        if h264_type == 7 || h264_type == 8 {
            return VideoCodec::H264;
        }
        
        // H.265 IRAP types are 16-21
        if h265_type >= 16 && h265_type <= 21 {
            return VideoCodec::H265;
        }
        
        // H.264 IDR is type 5
        if h264_type == 5 {
            return VideoCodec::H264;
        }
        
        VideoCodec::H264 // Default
    }

    /// Parse raw video data to extract frames and codec config
    /// 
    /// The data is expected to be in Annex B format (start codes) or
    /// length-prefixed NAL units. Works for both H.264 and H.265.
    pub fn parse_frames(&mut self, data: &[u8], base_timestamp: u32) -> Vec<H264Frame> {
        let mut frames = Vec::new();
        
        // Auto-detect codec if not set
        if self.h264_config.sps.is_empty() && self.h265_config.sps.is_empty() {
            self.codec = Self::detect_codec(data);
            debug!(codec = ?self.codec, "Auto-detected codec");
        }
        
        let nal_units = self.parse_nal_units(data);
        
        if nal_units.is_empty() {
            return frames;
        }

        // Extract parameter sets for codec config
        for nal in &nal_units {
            match (&self.codec, &nal.nal_type) {
                (VideoCodec::H264, NalUnitType::H264(H264NalType::Sps)) => {
                    if let Some((profile, compat, level, width, height)) = self.parse_h264_sps(&nal.data) {
                        self.h264_config.profile_idc = profile;
                        self.h264_config.profile_compat = compat;
                        self.h264_config.level_idc = level;
                        self.h264_config.width = width;
                        self.h264_config.height = height;
                    }
                    self.h264_config.sps.push(nal.data.clone());
                }
                (VideoCodec::H264, NalUnitType::H264(H264NalType::Pps)) => {
                    self.h264_config.pps.push(nal.data.clone());
                }
                (VideoCodec::H265, NalUnitType::H265(H265NalType::VpsNut)) => {
                    self.h265_config.vps.push(nal.data.clone());
                }
                (VideoCodec::H265, NalUnitType::H265(H265NalType::SpsNut)) => {
                    if let Some((width, height, profile_info)) = self.parse_h265_sps(&nal.data) {
                        self.h265_config.width = width;
                        self.h265_config.height = height;
                        if let Some((profile_space, tier, profile_idc, compat, constraints, level)) = profile_info {
                            self.h265_config.general_profile_space = profile_space;
                            self.h265_config.general_tier_flag = tier;
                            self.h265_config.general_profile_idc = profile_idc;
                            self.h265_config.general_profile_compatibility_flags = compat;
                            self.h265_config.general_constraint_indicator_flags = constraints;
                            self.h265_config.general_level_idc = level;
                        }
                    }
                    self.h265_config.sps.push(nal.data.clone());
                }
                (VideoCodec::H265, NalUnitType::H265(H265NalType::PpsNut)) => {
                    self.h265_config.pps.push(nal.data.clone());
                }
                _ => {}
            }
        }

        // Group NAL units into frames
        let mut current_frame_nals: Vec<NalUnit> = Vec::new();
        let mut frame_is_keyframe = false;
        let mut frame_timestamp = base_timestamp;
        let mut frame_count = 0u32;

        for nal in nal_units {
            let is_aud = match &nal.nal_type {
                NalUnitType::H264(H264NalType::Aud) => true,
                NalUnitType::H265(H265NalType::AudNut) => true,
                _ => false,
            };
            
            if is_aud {
                // Access Unit Delimiter marks start of new frame
                if !current_frame_nals.is_empty() {
                    let total_size = current_frame_nals.iter()
                        .map(|n| 4 + n.data.len()) // 4-byte length prefix
                        .sum();
                    
                    frames.push(H264Frame {
                        is_keyframe: frame_is_keyframe,
                        timestamp: frame_timestamp,
                        nal_units: std::mem::take(&mut current_frame_nals),
                        total_size,
                    });
                    
                    frame_count += 1;
                    frame_timestamp = base_timestamp + frame_count * 3000; // ~30fps at 90kHz
                    frame_is_keyframe = false;
                }
                continue;
            }
            
            // Check if this NAL indicates a keyframe
            if nal.nal_type.is_keyframe() {
                frame_is_keyframe = true;
            }
            
            // Skip parameter sets in mdat (they go in config box)
            if !nal.nal_type.is_parameter_set() {
                current_frame_nals.push(nal);
            }
        }

        // Don't forget the last frame
        if !current_frame_nals.is_empty() {
            let total_size = current_frame_nals.iter()
                .map(|n| 4 + n.data.len())
                .sum();
            
            frames.push(H264Frame {
                is_keyframe: frame_is_keyframe,
                timestamp: frame_timestamp,
                nal_units: current_frame_nals,
                total_size,
            });
        }

        frames
    }

    /// Parse NAL units from raw data (Annex B or length-prefixed)
    fn parse_nal_units(&self, data: &[u8]) -> Vec<NalUnit> {
        let mut units = Vec::new();
        
        if data.len() < 4 {
            return units;
        }

        // Check for Annex B start codes (0x00000001 or 0x000001)
        let is_annex_b = (data[0] == 0 && data[1] == 0 && data[2] == 0 && data[3] == 1)
            || (data[0] == 0 && data[1] == 0 && data[2] == 1);

        if is_annex_b {
            self.parse_annex_b(data, &mut units);
        } else {
            // Assume 4-byte length prefix (AVCC/HVCC format)
            self.parse_avcc(data, &mut units);
        }

        units
    }

    /// Parse Annex B format (start code delimited)
    fn parse_annex_b(&self, data: &[u8], units: &mut Vec<NalUnit>) {
        let mut i = 0;
        
        while i < data.len() {
            // Find start code
            let start = if i + 4 <= data.len() && data[i..i+4] == [0, 0, 0, 1] {
                i + 4
            } else if i + 3 <= data.len() && data[i..i+3] == [0, 0, 1] {
                i + 3
            } else {
                i += 1;
                continue;
            };

            // Find next start code or end
            let mut end = start;
            while end < data.len() {
                if end + 4 <= data.len() && data[end..end+4] == [0, 0, 0, 1] {
                    break;
                }
                if end + 3 <= data.len() && data[end..end+3] == [0, 0, 1] {
                    break;
                }
                end += 1;
            }

            if start < end && start < data.len() {
                let nal_data = &data[start..end];
                if !nal_data.is_empty() {
                    let nal_type = self.parse_nal_type(nal_data[0]);
                    units.push(NalUnit {
                        nal_type,
                        data: Bytes::copy_from_slice(nal_data),
                    });
                }
            }

            i = end;
        }
    }

    /// Parse AVCC/HVCC format (length-prefixed)
    fn parse_avcc(&self, data: &[u8], units: &mut Vec<NalUnit>) {
        let mut i = 0;
        
        while i + 4 <= data.len() {
            let length = u32::from_be_bytes([data[i], data[i+1], data[i+2], data[i+3]]) as usize;
            i += 4;
            
            if i + length > data.len() {
                warn!(expected = length, available = data.len() - i, "Truncated NAL unit");
                break;
            }

            let nal_data = &data[i..i+length];
            if !nal_data.is_empty() {
                let nal_type = self.parse_nal_type(nal_data[0]);
                units.push(NalUnit {
                    nal_type,
                    data: Bytes::copy_from_slice(nal_data),
                });
            }

            i += length;
        }
    }
    
    /// Parse NAL type based on detected codec
    fn parse_nal_type(&self, header: u8) -> NalUnitType {
        match self.codec {
            VideoCodec::H264 => NalUnitType::H264(H264NalType::from_header(header)),
            VideoCodec::H265 => NalUnitType::H265(H265NalType::from_header(header)),
        }
    }

    /// Parse H.264 SPS to extract profile/level and dimensions
    fn parse_h264_sps(&self, sps_data: &[u8]) -> Option<(u8, u8, u8, u32, u32)> {
        if sps_data.len() < 4 {
            return None;
        }

        let profile_idc = sps_data[1];
        let profile_compat = sps_data[2];
        let level_idc = sps_data[3];

        // Use h264-reader for proper SPS parsing
        let rbsp_data = &sps_data[1..]; // Skip NAL header
        let reader = BitReader::new(std::io::Cursor::new(rbsp_data));
        
        let (width, height) = match SeqParameterSet::from_bits(reader) {
            Ok(sps) => {
                match sps.pixel_dimensions() {
                    Ok((w, h)) => {
                        debug!(width = w, height = h, profile_idc, level_idc, "Parsed H.264 SPS");
                        (w, h)
                    }
                    Err(e) => {
                        warn!("Failed to extract dimensions from H.264 SPS: {:?}", e);
                        (1920, 1080)
                    }
                }
            }
            Err(e) => {
                warn!("Failed to parse H.264 SPS: {:?}", e);
                (1920, 1080)
            }
        };

        Some((profile_idc, profile_compat, level_idc, width, height))
    }
    
    /// Parse H.265 SPS to extract dimensions and profile info
    /// Returns (width, height, Option<(profile_space, tier, profile_idc, compat_flags, constraints, level)>)
    fn parse_h265_sps(&self, sps_data: &[u8]) -> Option<(u32, u32, Option<(u8, bool, u8, u32, u64, u8)>)> {
        if sps_data.len() < 4 {
            return None;
        }
        
        // H.265 SPS is more complex - we need to parse it manually or use retina's parser
        // For now, extract basic info from the raw bytes
        // H.265 SPS structure (after NAL header):
        // sps_video_parameter_set_id (4 bits)
        // sps_max_sub_layers_minus1 (3 bits)
        // sps_temporal_id_nesting_flag (1 bit)
        // profile_tier_level(...)
        // sps_seq_parameter_set_id (ue(v))
        // chroma_format_idc (ue(v))
        // if chroma_format_idc == 3: separate_colour_plane_flag (1 bit)
        // pic_width_in_luma_samples (ue(v))
        // pic_height_in_luma_samples (ue(v))
        // ...
        
        // For a robust implementation, we should use proper exp-golomb parsing
        // For now, try to extract profile_tier_level from fixed positions
        
        // NAL header for H.265 is 2 bytes
        if sps_data.len() < 15 {
            warn!("H.265 SPS too short");
            return Some((1920, 1080, None));
        }
        
        // After 2-byte NAL header + 1 byte (sps_video_parameter_set_id + sps_max_sub_layers_minus1 + flag)
        // comes profile_tier_level structure
        let ptl_start = 2; // Skip 2-byte NAL header
        
        // Read profile_tier_level (simplified - assumes sps_max_sub_layers_minus1 = 0)
        // general_profile_space (2 bits) + general_tier_flag (1 bit) + general_profile_idc (5 bits) = 1 byte
        // This is byte index 3 (after NAL header + sps info byte)
        let profile_byte = sps_data.get(ptl_start + 1)?;
        let general_profile_space = (profile_byte >> 6) & 0x03;
        let general_tier_flag = (profile_byte >> 5) & 0x01 != 0;
        let general_profile_idc = profile_byte & 0x1F;
        
        // general_profile_compatibility_flags (32 bits) = bytes 4-7
        let compat_flags = if sps_data.len() > ptl_start + 6 {
            u32::from_be_bytes([
                sps_data[ptl_start + 2],
                sps_data[ptl_start + 3],
                sps_data[ptl_start + 4],
                sps_data[ptl_start + 5],
            ])
        } else {
            0
        };
        
        // general_progressive_source_flag + constraints (48 bits) = bytes 8-13
        let constraints = if sps_data.len() > ptl_start + 12 {
            ((sps_data[ptl_start + 6] as u64) << 40)
                | ((sps_data[ptl_start + 7] as u64) << 32)
                | ((sps_data[ptl_start + 8] as u64) << 24)
                | ((sps_data[ptl_start + 9] as u64) << 16)
                | ((sps_data[ptl_start + 10] as u64) << 8)
                | (sps_data[ptl_start + 11] as u64)
        } else {
            0
        };
        
        // general_level_idc (8 bits)
        let general_level_idc = sps_data.get(ptl_start + 12).copied().unwrap_or(0);
        
        debug!(
            general_profile_idc,
            general_level_idc,
            "Parsed H.265 SPS profile/level"
        );
        
        // For dimensions, we need proper exp-golomb parsing
        // Default to common values for now - proper parsing would require more work
        Some((1920, 1080, Some((
            general_profile_space,
            general_tier_flag,
            general_profile_idc,
            compat_flags,
            constraints,
            general_level_idc,
        ))))
    }

    /// Add a frame to the muxer
    pub fn add_frame(&mut self, frame: &H264Frame) {
        // Calculate offset (will be updated when we know mdat start)
        let frame_offset = self.current_offset;
        
        // Write frame data in AVCC/HVCC format (4-byte length prefix)
        let frame_start = self.mdat_data.len();
        for nal in &frame.nal_units {
            // Skip parameter sets in mdat (they're in config box)
            if nal.nal_type.is_parameter_set() {
                continue;
            }
            
            // 4-byte length prefix
            self.mdat_data.put_u32(nal.data.len() as u32);
            self.mdat_data.put_slice(&nal.data);
        }
        let frame_size = self.mdat_data.len() - frame_start;

        if frame_size > 0 {
            self.samples.push(SampleEntry {
                size: frame_size as u32,
                duration: 3000, // ~30fps at 90kHz timescale
                is_sync: frame.is_keyframe,
                offset: frame_offset,
            });

            self.current_offset += frame_size as u64;
        }
    }

    /// Set H.264 codec config directly (from stream metadata)
    pub fn set_config(&mut self, sps: Bytes, pps: Bytes, width: u32, height: u32) {
        self.codec = VideoCodec::H264;
        if sps.len() >= 4 {
            self.h264_config.profile_idc = sps[1];
            self.h264_config.profile_compat = sps[2];
            self.h264_config.level_idc = sps[3];
        }
        self.h264_config.sps = vec![sps];
        self.h264_config.pps = vec![pps];
        self.h264_config.width = width;
        self.h264_config.height = height;
    }
    
    /// Set H.265 codec config directly (from stream metadata)
    pub fn set_h265_config(&mut self, vps: Bytes, sps: Bytes, pps: Bytes, width: u32, height: u32) {
        self.codec = VideoCodec::H265;
        self.h265_config.vps = vec![vps];
        self.h265_config.sps = vec![sps];
        self.h265_config.pps = vec![pps];
        self.h265_config.width = width;
        self.h265_config.height = height;
    }
    
    /// Get the current codec configuration
    pub fn get_codec_config(&self) -> CodecConfig {
        match self.codec {
            VideoCodec::H264 => CodecConfig::H264(self.h264_config.clone()),
            VideoCodec::H265 => CodecConfig::H265(self.h265_config.clone()),
        }
    }
    
    /// Get the detected video codec
    pub fn codec(&self) -> VideoCodec {
        self.codec
    }
    
    /// Get video dimensions
    pub fn dimensions(&self) -> (u32, u32) {
        match self.codec {
            VideoCodec::H264 => (self.h264_config.width, self.h264_config.height),
            VideoCodec::H265 => (self.h265_config.width, self.h265_config.height),
        }
    }

    /// Finalize and generate the MP4 file
    pub fn finalize(mut self) -> Bytes {
        if self.samples.is_empty() {
            warn!("No samples to mux");
            return Bytes::new();
        }

        // Calculate total duration
        let total_duration: u64 = self.samples.iter().map(|s| s.duration as u64).sum();
        
        // Build moov box first to know its size
        let moov = self.build_moov(total_duration);
        
        // Now we know the mdat offset (ftyp size + moov size + mdat header)
        let ftyp_size = 20u64;
        let mdat_header_size = 8u64;
        let mdat_offset = ftyp_size + moov.len() as u64 + mdat_header_size;
        
        // Update chunk offsets in samples
        for sample in &mut self.samples {
            sample.offset += mdat_offset;
        }
        
        // Rebuild moov with correct offsets
        let moov = self.build_moov(total_duration);
        
        // Build final output
        let mut output = BytesMut::with_capacity(
            20 + moov.len() + 8 + self.mdat_data.len()
        );
        
        // ftyp
        self.write_ftyp(&mut output);
        
        // moov
        output.put_slice(&moov);
        
        // mdat
        self.write_mdat(&mut output);
        
        debug!(
            samples = self.samples.len(),
            duration_secs = total_duration as f64 / self.timescale as f64,
            size = output.len(),
            "MP4 muxing complete"
        );
        
        output.freeze()
    }

    fn write_ftyp(&self, out: &mut BytesMut) {
        out.put_u32(20); // size
        out.put_slice(b"ftyp");
        out.put_slice(b"isom"); // major brand
        out.put_u32(0x200); // minor version
        out.put_slice(b"isom"); // compatible brand
    }

    fn write_mdat(&self, out: &mut BytesMut) {
        let size = 8 + self.mdat_data.len() as u32;
        out.put_u32(size);
        out.put_slice(b"mdat");
        out.put_slice(&self.mdat_data);
    }

    fn build_moov(&self, total_duration: u64) -> Bytes {
        let mut moov = BytesMut::new();
        
        // Build all sub-boxes
        let mvhd = self.build_mvhd(total_duration);
        let trak = self.build_trak(total_duration);
        
        let moov_size = 8 + mvhd.len() + trak.len();
        
        moov.put_u32(moov_size as u32);
        moov.put_slice(b"moov");
        moov.put_slice(&mvhd);
        moov.put_slice(&trak);
        
        moov.freeze()
    }

    fn build_mvhd(&self, total_duration: u64) -> Bytes {
        let mut mvhd = BytesMut::new();
        
        mvhd.put_u32(108); // size
        mvhd.put_slice(b"mvhd");
        mvhd.put_u32(0); // version & flags
        mvhd.put_u32(0); // creation time
        mvhd.put_u32(0); // modification time
        mvhd.put_u32(self.timescale); // timescale
        mvhd.put_u32(total_duration as u32); // duration
        mvhd.put_u32(0x00010000); // rate (1.0)
        mvhd.put_u16(0x0100); // volume (1.0)
        mvhd.put_slice(&[0u8; 10]); // reserved
        // Matrix (identity)
        mvhd.put_slice(&[
            0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x00,
        ]);
        mvhd.put_slice(&[0u8; 24]); // pre-defined
        mvhd.put_u32(2); // next track ID
        
        mvhd.freeze()
    }

    fn build_trak(&self, total_duration: u64) -> Bytes {
        let mut trak = BytesMut::new();
        
        let tkhd = self.build_tkhd(total_duration);
        let mdia = self.build_mdia(total_duration);
        
        let trak_size = 8 + tkhd.len() + mdia.len();
        
        trak.put_u32(trak_size as u32);
        trak.put_slice(b"trak");
        trak.put_slice(&tkhd);
        trak.put_slice(&mdia);
        
        trak.freeze()
    }

    fn build_tkhd(&self, total_duration: u64) -> Bytes {
        let mut tkhd = BytesMut::new();
        let (width, height) = self.dimensions();
        
        tkhd.put_u32(92); // size
        tkhd.put_slice(b"tkhd");
        tkhd.put_u32(0x00000003); // version 0, flags (enabled, in movie)
        tkhd.put_u32(0); // creation time
        tkhd.put_u32(0); // modification time
        tkhd.put_u32(1); // track ID
        tkhd.put_u32(0); // reserved
        tkhd.put_u32(total_duration as u32); // duration
        tkhd.put_slice(&[0u8; 8]); // reserved
        tkhd.put_u16(0); // layer
        tkhd.put_u16(0); // alternate group
        tkhd.put_u16(0); // volume
        tkhd.put_u16(0); // reserved
        // Matrix (identity)
        tkhd.put_slice(&[
            0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x00,
        ]);
        // Width and height as 16.16 fixed point
        tkhd.put_u32(width << 16);
        tkhd.put_u32(height << 16);
        
        tkhd.freeze()
    }

    fn build_mdia(&self, total_duration: u64) -> Bytes {
        let mut mdia = BytesMut::new();
        
        let mdhd = self.build_mdhd(total_duration);
        let hdlr = self.build_hdlr();
        let minf = self.build_minf();
        
        let mdia_size = 8 + mdhd.len() + hdlr.len() + minf.len();
        
        mdia.put_u32(mdia_size as u32);
        mdia.put_slice(b"mdia");
        mdia.put_slice(&mdhd);
        mdia.put_slice(&hdlr);
        mdia.put_slice(&minf);
        
        mdia.freeze()
    }

    fn build_mdhd(&self, total_duration: u64) -> Bytes {
        let mut mdhd = BytesMut::new();
        
        mdhd.put_u32(32); // size
        mdhd.put_slice(b"mdhd");
        mdhd.put_u32(0); // version & flags
        mdhd.put_u32(0); // creation time
        mdhd.put_u32(0); // modification time
        mdhd.put_u32(self.timescale); // timescale
        mdhd.put_u32(total_duration as u32); // duration
        mdhd.put_u16(0x55C4); // language (und)
        mdhd.put_u16(0); // pre-defined
        
        mdhd.freeze()
    }

    fn build_hdlr(&self) -> Bytes {
        let mut hdlr = BytesMut::new();
        
        let name = b"VideoHandler\0";
        hdlr.put_u32(32 + name.len() as u32); // size
        hdlr.put_slice(b"hdlr");
        hdlr.put_u32(0); // version & flags
        hdlr.put_u32(0); // pre-defined
        hdlr.put_slice(b"vide"); // handler type
        hdlr.put_slice(&[0u8; 12]); // reserved
        hdlr.put_slice(name);
        
        hdlr.freeze()
    }

    fn build_minf(&self) -> Bytes {
        let mut minf = BytesMut::new();
        
        let vmhd = self.build_vmhd();
        let dinf = self.build_dinf();
        let stbl = self.build_stbl();
        
        let minf_size = 8 + vmhd.len() + dinf.len() + stbl.len();
        
        minf.put_u32(minf_size as u32);
        minf.put_slice(b"minf");
        minf.put_slice(&vmhd);
        minf.put_slice(&dinf);
        minf.put_slice(&stbl);
        
        minf.freeze()
    }

    fn build_vmhd(&self) -> Bytes {
        let mut vmhd = BytesMut::new();
        
        vmhd.put_u32(20); // size
        vmhd.put_slice(b"vmhd");
        vmhd.put_u32(1); // version 0, flags (no lean ahead)
        vmhd.put_u16(0); // graphics mode
        vmhd.put_slice(&[0u8; 6]); // opcolor
        
        vmhd.freeze()
    }

    fn build_dinf(&self) -> Bytes {
        let mut dinf = BytesMut::new();
        
        // dref inside dinf
        let dref = self.build_dref();
        
        dinf.put_u32(8 + dref.len() as u32);
        dinf.put_slice(b"dinf");
        dinf.put_slice(&dref);
        
        dinf.freeze()
    }

    fn build_dref(&self) -> Bytes {
        let mut dref = BytesMut::new();
        
        // url box (self-contained)
        let url_box: &[u8] = &[
            0x00, 0x00, 0x00, 0x0C, // size
            b'u', b'r', b'l', b' ',
            0x00, 0x00, 0x00, 0x01, // flags (self-contained)
        ];
        
        dref.put_u32(8 + 4 + url_box.len() as u32);
        dref.put_slice(b"dref");
        dref.put_u32(0); // version & flags
        dref.put_u32(1); // entry count
        dref.put_slice(url_box);
        
        dref.freeze()
    }

    fn build_stbl(&self) -> Bytes {
        let mut stbl = BytesMut::new();
        
        let stsd = self.build_stsd();
        let stts = self.build_stts();
        let stss = self.build_stss();
        let stsc = self.build_stsc();
        let stsz = self.build_stsz();
        let stco = self.build_stco();
        
        let stbl_size = 8 + stsd.len() + stts.len() + stss.len() + stsc.len() + stsz.len() + stco.len();
        
        stbl.put_u32(stbl_size as u32);
        stbl.put_slice(b"stbl");
        stbl.put_slice(&stsd);
        stbl.put_slice(&stts);
        stbl.put_slice(&stss);
        stbl.put_slice(&stsc);
        stbl.put_slice(&stsz);
        stbl.put_slice(&stco);
        
        stbl.freeze()
    }

    fn build_stsd(&self) -> Bytes {
        let mut stsd = BytesMut::new();
        
        let sample_entry = match self.codec {
            VideoCodec::H264 => self.build_avc1(),
            VideoCodec::H265 => self.build_hvc1(),
        };
        
        stsd.put_u32(8 + 4 + 4 + sample_entry.len() as u32);
        stsd.put_slice(b"stsd");
        stsd.put_u32(0); // version & flags
        stsd.put_u32(1); // entry count
        stsd.put_slice(&sample_entry);
        
        stsd.freeze()
    }

    fn build_avc1(&self) -> Bytes {
        let mut avc1 = BytesMut::new();
        
        let avcc = self.build_avcc();
        let (width, height) = self.dimensions();
        
        let avc1_size = 8 + 78 + avcc.len();
        
        avc1.put_u32(avc1_size as u32);
        avc1.put_slice(b"avc1");
        avc1.put_slice(&[0u8; 6]); // reserved
        avc1.put_u16(1); // data reference index
        avc1.put_slice(&[0u8; 16]); // pre-defined + reserved
        avc1.put_u16(width as u16);
        avc1.put_u16(height as u16);
        avc1.put_u32(0x00480000); // horizontal resolution (72 dpi)
        avc1.put_u32(0x00480000); // vertical resolution (72 dpi)
        avc1.put_u32(0); // reserved
        avc1.put_u16(1); // frame count
        avc1.put_slice(&[0u8; 32]); // compressor name
        avc1.put_u16(0x0018); // depth (24 bit)
        avc1.put_i16(-1); // pre-defined
        avc1.put_slice(&avcc);
        
        avc1.freeze()
    }
    
    fn build_hvc1(&self) -> Bytes {
        let mut hvc1 = BytesMut::new();
        
        let hvcc = self.build_hvcc();
        let (width, height) = self.dimensions();
        
        let hvc1_size = 8 + 78 + hvcc.len();
        
        hvc1.put_u32(hvc1_size as u32);
        hvc1.put_slice(b"hvc1");
        hvc1.put_slice(&[0u8; 6]); // reserved
        hvc1.put_u16(1); // data reference index
        hvc1.put_slice(&[0u8; 16]); // pre-defined + reserved
        hvc1.put_u16(width as u16);
        hvc1.put_u16(height as u16);
        hvc1.put_u32(0x00480000); // horizontal resolution (72 dpi)
        hvc1.put_u32(0x00480000); // vertical resolution (72 dpi)
        hvc1.put_u32(0); // reserved
        hvc1.put_u16(1); // frame count
        hvc1.put_slice(&[0u8; 32]); // compressor name
        hvc1.put_u16(0x0018); // depth (24 bit)
        hvc1.put_i16(-1); // pre-defined
        hvc1.put_slice(&hvcc);
        
        hvc1.freeze()
    }

    fn build_avcc(&self) -> Bytes {
        let mut avcc = BytesMut::new();
        
        // Calculate total size
        let sps_total: usize = self.h264_config.sps.iter().map(|s| 2 + s.len()).sum();
        let pps_total: usize = self.h264_config.pps.iter().map(|p| 2 + p.len()).sum();
        let avcc_size = 8 + 7 + sps_total + 1 + pps_total;
        
        avcc.put_u32(avcc_size as u32);
        avcc.put_slice(b"avcC");
        avcc.put_u8(1); // configuration version
        avcc.put_u8(self.h264_config.profile_idc);
        avcc.put_u8(self.h264_config.profile_compat);
        avcc.put_u8(self.h264_config.level_idc);
        avcc.put_u8(0xFF); // length size minus one (3 = 4 bytes) | reserved
        avcc.put_u8(0xE0 | self.h264_config.sps.len() as u8); // num SPS | reserved
        
        for sps in &self.h264_config.sps {
            avcc.put_u16(sps.len() as u16);
            avcc.put_slice(sps);
        }
        
        avcc.put_u8(self.h264_config.pps.len() as u8); // num PPS
        
        for pps in &self.h264_config.pps {
            avcc.put_u16(pps.len() as u16);
            avcc.put_slice(pps);
        }
        
        avcc.freeze()
    }
    
    /// Build hvcC (HEVC decoder configuration record) box
    fn build_hvcc(&self) -> Bytes {
        let mut hvcc = BytesMut::new();
        
        // HEVCDecoderConfigurationRecord structure:
        // - configurationVersion (8 bits)
        // - general_profile_space (2) + general_tier_flag (1) + general_profile_idc (5)
        // - general_profile_compatibility_flags (32 bits)
        // - general_constraint_indicator_flags (48 bits)
        // - general_level_idc (8 bits)
        // - reserved (4) + min_spatial_segmentation_idc (12 bits)
        // - reserved (6) + parallelismType (2 bits)
        // - reserved (6) + chroma_format_idc (2 bits)
        // - reserved (5) + bit_depth_luma_minus8 (3 bits)
        // - reserved (5) + bit_depth_chroma_minus8 (3 bits)
        // - avgFrameRate (16 bits)
        // - constantFrameRate (2) + numTemporalLayers (3) + temporalIdNested (1) + lengthSizeMinusOne (2)
        // - numOfArrays (8 bits)
        // - arrays (variable)
        
        let config = &self.h265_config;
        
        // Calculate sizes for arrays
        let vps_total: usize = config.vps.iter().map(|v| 2 + v.len()).sum();
        let sps_total: usize = config.sps.iter().map(|s| 2 + s.len()).sum();
        let pps_total: usize = config.pps.iter().map(|p| 2 + p.len()).sum();
        
        // Each array has: array_completeness (1) + reserved (1) + NAL_unit_type (6) + numNalus (16) + nalus
        let num_arrays = (if config.vps.is_empty() { 0 } else { 1 })
            + (if config.sps.is_empty() { 0 } else { 1 })
            + (if config.pps.is_empty() { 0 } else { 1 });
        
        let array_overhead = num_arrays * 3; // 1 byte type + 2 bytes count per array
        let hvcc_size = 8 + 23 + array_overhead + vps_total + sps_total + pps_total;
        
        hvcc.put_u32(hvcc_size as u32);
        hvcc.put_slice(b"hvcC");
        
        // configurationVersion = 1
        hvcc.put_u8(1);
        
        // general_profile_space (2) + general_tier_flag (1) + general_profile_idc (5)
        let profile_byte = ((config.general_profile_space & 0x03) << 6)
            | (if config.general_tier_flag { 0x20 } else { 0 })
            | (config.general_profile_idc & 0x1F);
        hvcc.put_u8(profile_byte);
        
        // general_profile_compatibility_flags (32 bits)
        hvcc.put_u32(config.general_profile_compatibility_flags);
        
        // general_constraint_indicator_flags (48 bits) - stored as 6 bytes
        hvcc.put_u8(((config.general_constraint_indicator_flags >> 40) & 0xFF) as u8);
        hvcc.put_u8(((config.general_constraint_indicator_flags >> 32) & 0xFF) as u8);
        hvcc.put_u8(((config.general_constraint_indicator_flags >> 24) & 0xFF) as u8);
        hvcc.put_u8(((config.general_constraint_indicator_flags >> 16) & 0xFF) as u8);
        hvcc.put_u8(((config.general_constraint_indicator_flags >> 8) & 0xFF) as u8);
        hvcc.put_u8((config.general_constraint_indicator_flags & 0xFF) as u8);
        
        // general_level_idc
        hvcc.put_u8(config.general_level_idc);
        
        // reserved (4) + min_spatial_segmentation_idc (12) - set to 0
        hvcc.put_u16(0xF000); // reserved bits set to 1
        
        // reserved (6) + parallelismType (2)
        hvcc.put_u8(0xFC); // reserved + parallelismType=0
        
        // reserved (6) + chroma_format_idc (2)
        hvcc.put_u8(0xFC | (config.chroma_format_idc & 0x03));
        
        // reserved (5) + bit_depth_luma_minus8 (3)
        hvcc.put_u8(0xF8 | (config.bit_depth_luma_minus8 & 0x07));
        
        // reserved (5) + bit_depth_chroma_minus8 (3)
        hvcc.put_u8(0xF8 | (config.bit_depth_chroma_minus8 & 0x07));
        
        // avgFrameRate (0 = unspecified)
        hvcc.put_u16(0);
        
        // constantFrameRate (2) + numTemporalLayers (3) + temporalIdNested (1) + lengthSizeMinusOne (2)
        // 00 + 001 + 1 + 11 = 0x0F (1 temporal layer, nested, 4-byte NAL length)
        hvcc.put_u8(0x0F);
        
        // numOfArrays
        hvcc.put_u8(num_arrays as u8);
        
        // VPS array
        if !config.vps.is_empty() {
            hvcc.put_u8(0x80 | 32); // array_completeness=1, NAL_unit_type=32 (VPS)
            hvcc.put_u16(config.vps.len() as u16);
            for vps in &config.vps {
                hvcc.put_u16(vps.len() as u16);
                hvcc.put_slice(vps);
            }
        }
        
        // SPS array
        if !config.sps.is_empty() {
            hvcc.put_u8(0x80 | 33); // array_completeness=1, NAL_unit_type=33 (SPS)
            hvcc.put_u16(config.sps.len() as u16);
            for sps in &config.sps {
                hvcc.put_u16(sps.len() as u16);
                hvcc.put_slice(sps);
            }
        }
        
        // PPS array
        if !config.pps.is_empty() {
            hvcc.put_u8(0x80 | 34); // array_completeness=1, NAL_unit_type=34 (PPS)
            hvcc.put_u16(config.pps.len() as u16);
            for pps in &config.pps {
                hvcc.put_u16(pps.len() as u16);
                hvcc.put_slice(pps);
            }
        }
        
        hvcc.freeze()
    }

    fn build_stts(&self) -> Bytes {
        let mut stts = BytesMut::new();
        
        // Group samples with same duration
        let mut entries: Vec<(u32, u32)> = Vec::new(); // (count, delta)
        
        for sample in &self.samples {
            if let Some((count, delta)) = entries.last_mut() {
                if *delta == sample.duration {
                    *count += 1;
                    continue;
                }
            }
            entries.push((1, sample.duration));
        }
        
        let stts_size = 8 + 4 + 4 + entries.len() * 8;
        
        stts.put_u32(stts_size as u32);
        stts.put_slice(b"stts");
        stts.put_u32(0); // version & flags
        stts.put_u32(entries.len() as u32);
        
        for (count, delta) in entries {
            stts.put_u32(count);
            stts.put_u32(delta);
        }
        
        stts.freeze()
    }

    fn build_stss(&self) -> Bytes {
        let mut stss = BytesMut::new();
        
        // Sync samples (keyframes) - 1-indexed
        let sync_samples: Vec<u32> = self.samples.iter()
            .enumerate()
            .filter(|(_, s)| s.is_sync)
            .map(|(i, _)| i as u32 + 1)
            .collect();
        
        let stss_size = 8 + 4 + 4 + sync_samples.len() * 4;
        
        stss.put_u32(stss_size as u32);
        stss.put_slice(b"stss");
        stss.put_u32(0); // version & flags
        stss.put_u32(sync_samples.len() as u32);
        
        for sample_num in sync_samples {
            stss.put_u32(sample_num);
        }
        
        stss.freeze()
    }

    fn build_stsc(&self) -> Bytes {
        let mut stsc = BytesMut::new();
        
        // Simple: one sample per chunk
        stsc.put_u32(8 + 4 + 4 + 12); // size
        stsc.put_slice(b"stsc");
        stsc.put_u32(0); // version & flags
        stsc.put_u32(1); // entry count
        stsc.put_u32(1); // first chunk
        stsc.put_u32(1); // samples per chunk
        stsc.put_u32(1); // sample description index
        
        stsc.freeze()
    }

    fn build_stsz(&self) -> Bytes {
        let mut stsz = BytesMut::new();
        
        let stsz_size = 8 + 4 + 4 + 4 + self.samples.len() * 4;
        
        stsz.put_u32(stsz_size as u32);
        stsz.put_slice(b"stsz");
        stsz.put_u32(0); // version & flags
        stsz.put_u32(0); // sample size (0 = variable)
        stsz.put_u32(self.samples.len() as u32);
        
        for sample in &self.samples {
            stsz.put_u32(sample.size);
        }
        
        stsz.freeze()
    }

    fn build_stco(&self) -> Bytes {
        let mut stco = BytesMut::new();
        
        // Use co64 if offsets exceed 32-bit
        let use_co64 = self.samples.iter().any(|s| s.offset > u32::MAX as u64);
        
        if use_co64 {
            let co64_size = 8 + 4 + 4 + self.samples.len() * 8;
            
            stco.put_u32(co64_size as u32);
            stco.put_slice(b"co64");
            stco.put_u32(0); // version & flags
            stco.put_u32(self.samples.len() as u32);
            
            for sample in &self.samples {
                stco.put_u64(sample.offset);
            }
        } else {
            let stco_size = 8 + 4 + 4 + self.samples.len() * 4;
            
            stco.put_u32(stco_size as u32);
            stco.put_slice(b"stco");
            stco.put_u32(0); // version & flags
            stco.put_u32(self.samples.len() as u32);
            
            for sample in &self.samples {
                stco.put_u32(sample.offset as u32);
            }
        }
        
        stco.freeze()
    }
}

impl Default for Mp4Muxer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_h264_nal_unit_type_parsing() {
        // Test H.264 NAL type parsing (5-bit type in lower bits)
        assert_eq!(H264NalType::from_header(0x67), H264NalType::Sps);     // 0x67 & 0x1F = 7 (SPS)
        assert_eq!(H264NalType::from_header(0x68), H264NalType::Pps);     // 0x68 & 0x1F = 8 (PPS)
        assert_eq!(H264NalType::from_header(0x65), H264NalType::Idr);     // 0x65 & 0x1F = 5 (IDR)
        assert_eq!(H264NalType::from_header(0x41), H264NalType::Slice);   // 0x41 & 0x1F = 1 (Slice)
        assert_eq!(H264NalType::from_header(0x09), H264NalType::Aud);     // 0x09 & 0x1F = 9 (AUD)
    }
    
    #[test]
    fn test_h265_nal_unit_type_parsing() {
        // Test H.265 NAL type parsing (6-bit type shifted right by 1)
        assert_eq!(H265NalType::from_header(0x40), H265NalType::VpsNut);  // (0x40 >> 1) & 0x3F = 32 (VPS)
        assert_eq!(H265NalType::from_header(0x42), H265NalType::SpsNut);  // (0x42 >> 1) & 0x3F = 33 (SPS)
        assert_eq!(H265NalType::from_header(0x44), H265NalType::PpsNut);  // (0x44 >> 1) & 0x3F = 34 (PPS)
        assert_eq!(H265NalType::from_header(0x26), H265NalType::IdrWRadl); // (0x26 >> 1) & 0x3F = 19 (IDR_W_RADL)
        assert_eq!(H265NalType::from_header(0x46), H265NalType::AudNut);  // (0x46 >> 1) & 0x3F = 35 (AUD)
    }

    #[test]
    fn test_muxer_creation() {
        let muxer = Mp4Muxer::new();
        assert_eq!(muxer.timescale, 90000);
        assert!(muxer.samples.is_empty());
        assert_eq!(muxer.codec(), VideoCodec::H264); // Default
    }
    
    #[test]
    fn test_muxer_with_codec() {
        let muxer = Mp4Muxer::with_codec(VideoCodec::H265);
        assert_eq!(muxer.codec(), VideoCodec::H265);
    }

    #[test]
    fn test_parse_h264_annex_b() {
        let mut muxer = Mp4Muxer::new();
        muxer.codec = VideoCodec::H264;
        
        // H.264 Annex B data with start codes
        let data = [
            0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x1E, // SPS
            0x00, 0x00, 0x00, 0x01, 0x68, 0xCE, 0x38, 0x80, // PPS
            0x00, 0x00, 0x00, 0x01, 0x65, 0x88, 0x84, 0x00, // IDR
        ];
        
        let mut units = Vec::new();
        muxer.parse_annex_b(&data, &mut units);
        
        assert_eq!(units.len(), 3);
        assert_eq!(units[0].nal_type, NalUnitType::H264(H264NalType::Sps));
        assert_eq!(units[1].nal_type, NalUnitType::H264(H264NalType::Pps));
        assert_eq!(units[2].nal_type, NalUnitType::H264(H264NalType::Idr));
    }
    
    #[test]
    fn test_parse_h265_annex_b() {
        let mut muxer = Mp4Muxer::new();
        muxer.codec = VideoCodec::H265;
        
        // H.265 Annex B data with start codes (2-byte NAL headers)
        let data = [
            0x00, 0x00, 0x00, 0x01, 0x40, 0x01, 0x0C, 0x01, // VPS (type 32)
            0x00, 0x00, 0x00, 0x01, 0x42, 0x01, 0x01, 0x01, // SPS (type 33)
            0x00, 0x00, 0x00, 0x01, 0x44, 0x01, 0xC1, 0x73, // PPS (type 34)
        ];
        
        let mut units = Vec::new();
        muxer.parse_annex_b(&data, &mut units);
        
        assert_eq!(units.len(), 3);
        assert_eq!(units[0].nal_type, NalUnitType::H265(H265NalType::VpsNut));
        assert_eq!(units[1].nal_type, NalUnitType::H265(H265NalType::SpsNut));
        assert_eq!(units[2].nal_type, NalUnitType::H265(H265NalType::PpsNut));
    }

    #[test]
    fn test_parse_h264_avcc() {
        let mut muxer = Mp4Muxer::new();
        muxer.codec = VideoCodec::H264;
        
        // AVCC data with length prefixes
        let data = [
            0x00, 0x00, 0x00, 0x04, 0x67, 0x42, 0x00, 0x1E, // SPS (len=4)
            0x00, 0x00, 0x00, 0x04, 0x68, 0xCE, 0x38, 0x80, // PPS (len=4)
        ];
        
        let mut units = Vec::new();
        muxer.parse_avcc(&data, &mut units);
        
        assert_eq!(units.len(), 2);
        assert_eq!(units[0].nal_type, NalUnitType::H264(H264NalType::Sps));
        assert_eq!(units[1].nal_type, NalUnitType::H264(H264NalType::Pps));
    }

    #[test]
    fn test_ftyp_box() {
        let muxer = Mp4Muxer::new();
        let mut buf = BytesMut::new();
        muxer.write_ftyp(&mut buf);
        
        assert_eq!(buf.len(), 20);
        assert_eq!(&buf[4..8], b"ftyp");
        assert_eq!(&buf[8..12], b"isom");
    }

    #[test]
    fn test_avcc_building() {
        let mut muxer = Mp4Muxer::new();
        muxer.h264_config.sps = vec![Bytes::from_static(&[0x67, 0x42, 0x00, 0x1E])];
        muxer.h264_config.pps = vec![Bytes::from_static(&[0x68, 0xCE, 0x38, 0x80])];
        
        let avcc = muxer.build_avcc();
        
        assert_eq!(&avcc[4..8], b"avcC");
        assert_eq!(avcc[8], 1); // configuration version
    }
    
    #[test]
    fn test_hvcc_building() {
        let mut muxer = Mp4Muxer::with_codec(VideoCodec::H265);
        muxer.h265_config.vps = vec![Bytes::from_static(&[0x40, 0x01, 0x0C, 0x01])];
        muxer.h265_config.sps = vec![Bytes::from_static(&[0x42, 0x01, 0x01, 0x01])];
        muxer.h265_config.pps = vec![Bytes::from_static(&[0x44, 0x01, 0xC1, 0x73])];
        muxer.h265_config.general_profile_idc = 1; // Main profile
        muxer.h265_config.general_level_idc = 120; // Level 4.0
        
        let hvcc = muxer.build_hvcc();
        
        assert_eq!(&hvcc[4..8], b"hvcC");
        assert_eq!(hvcc[8], 1); // configuration version
    }

    #[test]
    fn test_full_mux_empty() {
        let muxer = Mp4Muxer::new();
        let output = muxer.finalize();
        
        // Should return empty for no samples
        assert!(output.is_empty());
    }

    #[test]
    fn test_add_h264_frame_and_finalize() {
        let mut muxer = Mp4Muxer::new();
        
        // Set up H.264 codec config
        muxer.set_config(
            Bytes::from_static(&[0x67, 0x42, 0x00, 0x1E, 0x95, 0xA8, 0x28]),
            Bytes::from_static(&[0x68, 0xCE, 0x38, 0x80]),
            1920,
            1080,
        );
        
        // Add a keyframe
        let frame = H264Frame {
            is_keyframe: true,
            timestamp: 0,
            nal_units: vec![
                NalUnit {
                    nal_type: NalUnitType::H264(H264NalType::Idr),
                    data: Bytes::from_static(&[0x65, 0x88, 0x84, 0x00, 0xFF, 0xFF]),
                }
            ],
            total_size: 10,
        };
        muxer.add_frame(&frame);
        
        // Add a P-frame
        let frame2 = H264Frame {
            is_keyframe: false,
            timestamp: 3000,
            nal_units: vec![
                NalUnit {
                    nal_type: NalUnitType::H264(H264NalType::Slice),
                    data: Bytes::from_static(&[0x41, 0x9A, 0x00, 0xFF]),
                }
            ],
            total_size: 8,
        };
        muxer.add_frame(&frame2);
        
        let output = muxer.finalize();
        
        // Should have produced valid MP4
        assert!(!output.is_empty());
        assert_eq!(&output[4..8], b"ftyp");
        
        // Find moov box
        let moov_pos = output.windows(4).position(|w| w == b"moov");
        assert!(moov_pos.is_some());
        
        // Find mdat box
        let mdat_pos = output.windows(4).position(|w| w == b"mdat");
        assert!(mdat_pos.is_some());
        
        // Find avc1 box (H.264)
        let avc1_pos = output.windows(4).position(|w| w == b"avc1");
        assert!(avc1_pos.is_some(), "H.264 should produce avc1 sample entry");
    }
    
    #[test]
    fn test_add_h265_frame_and_finalize() {
        let mut muxer = Mp4Muxer::with_codec(VideoCodec::H265);
        
        // Set up H.265 codec config
        muxer.set_h265_config(
            Bytes::from_static(&[0x40, 0x01, 0x0C, 0x01, 0xFF, 0xFF, 0x01]),
            Bytes::from_static(&[0x42, 0x01, 0x01, 0x01, 0x60, 0x00, 0x00]),
            Bytes::from_static(&[0x44, 0x01, 0xC1, 0x73, 0xD1, 0x89]),
            1920,
            1080,
        );
        muxer.h265_config.general_profile_idc = 1;
        muxer.h265_config.general_level_idc = 120;
        
        // Add an IDR frame (IRAP)
        let frame = H264Frame {
            is_keyframe: true,
            timestamp: 0,
            nal_units: vec![
                NalUnit {
                    nal_type: NalUnitType::H265(H265NalType::IdrWRadl),
                    data: Bytes::from_static(&[0x26, 0x01, 0xAF, 0x00, 0xFF, 0xFF]),
                }
            ],
            total_size: 10,
        };
        muxer.add_frame(&frame);
        
        let output = muxer.finalize();
        
        // Should have produced valid MP4
        assert!(!output.is_empty());
        assert_eq!(&output[4..8], b"ftyp");
        
        // Find hvc1 box (H.265)
        let hvc1_pos = output.windows(4).position(|w| w == b"hvc1");
        assert!(hvc1_pos.is_some(), "H.265 should produce hvc1 sample entry");
    }
    
    #[test]
    fn test_keyframe_detection() {
        // H.264 keyframe detection
        assert!(NalUnitType::H264(H264NalType::Idr).is_keyframe());
        assert!(!NalUnitType::H264(H264NalType::Slice).is_keyframe());
        
        // H.265 keyframe detection (all IRAP types)
        assert!(NalUnitType::H265(H265NalType::IdrWRadl).is_keyframe());
        assert!(NalUnitType::H265(H265NalType::IdrNLp).is_keyframe());
        assert!(NalUnitType::H265(H265NalType::CraNut).is_keyframe());
        assert!(!NalUnitType::H265(H265NalType::TrailR).is_keyframe());
    }
    
    #[test]
    fn test_parameter_set_detection() {
        // H.264 parameter sets
        assert!(NalUnitType::H264(H264NalType::Sps).is_parameter_set());
        assert!(NalUnitType::H264(H264NalType::Pps).is_parameter_set());
        assert!(!NalUnitType::H264(H264NalType::Idr).is_parameter_set());
        
        // H.265 parameter sets
        assert!(NalUnitType::H265(H265NalType::VpsNut).is_parameter_set());
        assert!(NalUnitType::H265(H265NalType::SpsNut).is_parameter_set());
        assert!(NalUnitType::H265(H265NalType::PpsNut).is_parameter_set());
        assert!(!NalUnitType::H265(H265NalType::IdrWRadl).is_parameter_set());
    }
}
