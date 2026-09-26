// Copyright 2011-2020, Molecular Matters GmbH <office@molecular-matters.com>
// See LICENSE.txt for licensing details (2-clause BSD License: https://opensource.org/licenses/BSD-2-Clause)

#include "PsdPch.h"
#include "PsdParseImageDataSection.h"

#include "PsdImageDataSection.h"
#include "PsdDocument.h"
#include "PsdCompressionType.h"
#include "PsdPlanarImage.h"
#include "PsdFile.h"
#include "PsdAllocator.h"
#include "PsdEndianConversion.h"
#include "PsdSyncFileReader.h"
#include "PsdSyncFileUtil.h"
#include "PsdMemoryUtil.h"
#include "PsdDecompressRle.h"
#include "PsdAssert.h"
#include "PsdLog.h"


PSD_NAMESPACE_BEGIN

namespace
{
	// ---------------------------------------------------------------------------------------------------------------------
	// ---------------------------------------------------------------------------------------------------------------------
	template <typename T>
	void EndianConvert(PlanarImage* images, unsigned int width, unsigned int height, unsigned int channelCount)
	{
		PSD_ASSERT_NOT_NULL(images);

		const unsigned int size = width*height;
		for (unsigned int i=0; i < channelCount; ++i)
		{
			T* planarData = static_cast<T*>(images[i].data);
			for (unsigned int j=0; j < size; ++j)
			{
				planarData[j] = endianUtil::BigEndianToNative(planarData[j]);
			}
		}
	}


	// review-psd patch (see ../NOTICE.txt): the two readers below size every buffer with
	// 64-bit arithmetic, refuse a plane that would not fit the 32-bit counts psd_sdk reads
	// with, check every allocation, and bound the RLE payload by the section it sits in. On
	// any failure they free what they allocated and return nullptr rather than writing
	// through a null or undersized buffer.
	static ImageDataSection* AllocateImageDataSection(Allocator* allocator, unsigned int channelCount)
	{
		ImageDataSection* imageData = memoryUtil::Allocate<ImageDataSection>(allocator);
		if (!imageData)
			return nullptr;

		imageData->imageCount = 0u;
		imageData->images = memoryUtil::AllocateArray<PlanarImage>(allocator, channelCount);
		if (!imageData->images)
		{
			memoryUtil::Free(allocator, imageData);
			return nullptr;
		}
		for (unsigned int i=0; i < channelCount; ++i)
		{
			imageData->images[i].data = nullptr;
		}
		imageData->imageCount = channelCount;
		return imageData;
	}


	// ---------------------------------------------------------------------------------------------------------------------
	// ---------------------------------------------------------------------------------------------------------------------
	static ImageDataSection* FreeImageDataSection(ImageDataSection* imageData, Allocator* allocator)
	{
		for (unsigned int i=0; i < imageData->imageCount; ++i)
		{
			allocator->Free(imageData->images[i].data);
		}
		memoryUtil::FreeArray(allocator, imageData->images);
		memoryUtil::Free(allocator, imageData);
		return nullptr;
	}


	// ---------------------------------------------------------------------------------------------------------------------
	// ---------------------------------------------------------------------------------------------------------------------
	static bool PlaneBytes(unsigned int width, unsigned int height, unsigned int bytesPerPixel, uint32_t* out)
	{
		const uint64_t bytes = static_cast<uint64_t>(width) * height * bytesPerPixel;
		if (bytes == 0u || bytes > UINT32_MAX)
			return false;
		*out = static_cast<uint32_t>(bytes);
		return true;
	}


	// ---------------------------------------------------------------------------------------------------------------------
	// ---------------------------------------------------------------------------------------------------------------------
	static ImageDataSection* ReadImageDataSectionRaw(SyncFileReader& reader, Allocator* allocator, unsigned int width, unsigned int height, unsigned int channelCount, unsigned int bytesPerPixel)
	{
		uint32_t planeBytes = 0u;
		if (channelCount == 0u || !PlaneBytes(width, height, bytesPerPixel, &planeBytes))
			return nullptr;

		ImageDataSection* imageData = AllocateImageDataSection(allocator, channelCount);
		if (!imageData)
			return nullptr;

		// read data for all channels at once
		for (unsigned int i=0; i < channelCount; ++i)
		{
			void* planarData = allocator->Allocate(planeBytes, 16u);
			if (!planarData)
				return FreeImageDataSection(imageData, allocator);
			imageData->images[i].data = planarData;

			reader.Read(planarData, planeBytes);
		}

		return imageData;
	}


	// ---------------------------------------------------------------------------------------------------------------------
	// ---------------------------------------------------------------------------------------------------------------------
	static ImageDataSection* ReadImageDataSectionRLE(SyncFileReader& reader, Allocator* allocator, unsigned int width, unsigned int height, unsigned int channelCount, unsigned int bytesPerPixel, uint64_t sectionLength)
	{
		// the RLE-compressed data is preceded by a 2-byte data count for each scan line, per channel.
		// we store the size of the RLE data per channel, and assume a maximum of 256 channels.
		if (channelCount == 0u || channelCount >= 256u)
		{
			PSD_ERROR("ImageData", "Image data section has an unsupported channel count (%u).", channelCount);
			return nullptr;
		}
		uint32_t planeBytes = 0u;
		if (!PlaneBytes(width, height, bytesPerPixel, &planeBytes))
			return nullptr;

		unsigned int channelSize[256] = {};
		uint64_t totalSize = 0u;
		for (unsigned int i=0; i < channelCount; ++i)
		{
			uint64_t size = 0u;
			for (unsigned int j=0; j < height; ++j)
			{
				const uint16_t dataCount = fileUtil::ReadFromFileBE<uint16_t>(reader);
				size += dataCount;
			}

			if (size > UINT32_MAX)
				return nullptr;
			channelSize[i] = static_cast<unsigned int>(size);
			totalSize += size;
		}

		// The compressed payload cannot be larger than the section holding it, whatever the
		// scan-line counts claim - refusing it here is what keeps a 40-byte file from asking
		// for gigabytes of RLE buffer.
		if (totalSize == 0u || totalSize > sectionLength)
			return nullptr;

		ImageDataSection* imageData = AllocateImageDataSection(allocator, channelCount);
		if (!imageData)
			return nullptr;

		for (unsigned int i=0; i < channelCount; ++i)
		{
			void* planarData = allocator->Allocate(planeBytes, 16u);
			if (!planarData)
				return FreeImageDataSection(imageData, allocator);
			imageData->images[i].data = planarData;

			// read RLE data, and uncompress into planar buffer
			const unsigned int rleSize = channelSize[i];
			if (rleSize == 0u)
				return FreeImageDataSection(imageData, allocator);
			uint8_t* rleData = static_cast<uint8_t*>(allocator->Allocate(rleSize, 4u));
			if (!rleData)
				return FreeImageDataSection(imageData, allocator);
			reader.Read(rleData, rleSize);

			const bool decoded = imageUtil::DecompressRle(rleData, rleSize, static_cast<uint8_t*>(planarData), planeBytes);

			allocator->Free(rleData);
			if (!decoded)
				return FreeImageDataSection(imageData, allocator);
		}

		return imageData;
	}
}


// ---------------------------------------------------------------------------------------------------------------------
// ---------------------------------------------------------------------------------------------------------------------
ImageDataSection* ParseImageDataSection(const Document* document, File* file, Allocator* allocator)
{
	PSD_ASSERT_NOT_NULL(file);
	PSD_ASSERT_NOT_NULL(allocator);

	// this is the merged image. it is only stored if "maximize compatibility" is turned on when saving a PSD file.
	// image data is stored in planar order: first red data, then green data, and so on.
	// each plane is stored in scan-line order, with no padding bytes.

	// 8-bit values are stored directly.
	// 16-bit values are stored directly, even though they are stored as 15-bit+1 integers in the range 0...32768
	// internally in Photoshop, see https://forums.adobe.com/message/3472269
	// 32-bit values are stored directly as IEEE 32-bit floats.
	const Section& section = document->imageDataSection;
	if (section.length == 0)
	{
		PSD_ERROR("PSD", "Document does not contain an image data section.");
		return nullptr;
	}

	SyncFileReader reader(file);
	reader.SetPosition(section.offset);

	ImageDataSection* imageData = nullptr;
	const unsigned int width = document->width;
	const unsigned int height = document->height;
	const unsigned int bitsPerChannel = document->bitsPerChannel;
	const unsigned int channelCount = document->channelCount;
	const uint16_t compressionType = fileUtil::ReadFromFileBE<uint16_t>(reader);
	if (compressionType == compressionType::RAW)
	{
		imageData = ReadImageDataSectionRaw(reader, allocator, width, height, channelCount, bitsPerChannel / 8u);
	}
	else if (compressionType == compressionType::RLE)
	{
		imageData = ReadImageDataSectionRLE(reader, allocator, width, height, channelCount, bitsPerChannel / 8u, section.length);
	}
	else
	{
		PSD_ERROR("ImageData", "Unhandled compression type %u.", compressionType);
	}

	if (!imageData)
		return nullptr;

	if (!imageData->images)
		return imageData;

	// endian-convert the data
	switch (bitsPerChannel)
	{
		case 8:
			EndianConvert<uint8_t>(imageData->images, width, height, channelCount);
			break;

		case 16:
			EndianConvert<uint16_t>(imageData->images, width, height, channelCount);
			break;

		case 32:
			EndianConvert<float32_t>(imageData->images, width, height, channelCount);
			break;

		default:
			PSD_ERROR("ImageData", "Unhandled bits per channel: %u.", bitsPerChannel);
			break;
	}

	return imageData;
}


// ---------------------------------------------------------------------------------------------------------------------
// ---------------------------------------------------------------------------------------------------------------------
void DestroyImageDataSection(ImageDataSection*& section, Allocator* allocator)
{
	PSD_ASSERT_NOT_NULL(section);
	PSD_ASSERT_NOT_NULL(allocator);

	for (unsigned int i=0; i < section->imageCount; ++i)
	{
		allocator->Free(section->images[i].data);
	}

	memoryUtil::FreeArray(allocator, section->images);
	memoryUtil::Free(allocator, section);
}

PSD_NAMESPACE_END
