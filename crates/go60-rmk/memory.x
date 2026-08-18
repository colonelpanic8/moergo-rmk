MEMORY
{
  /* Go60 uses the same MoErgo nRF52840 bootloader/storage boundaries as the
   * Glove80. Keep 0xdc000-0xec000 unused for runtime configuration so this
   * image cannot collide with either the settings or bootloader partitions.
   */
  FLASH : ORIGIN = 0x00026000, LENGTH = 0xB6000
  /* The last 256 bytes of app RAM (0x2003FB08..0x2003FC08) are the panic
   * store: a fixed cross-build address, so a stable image can read the
   * report a crashing image persisted. Owned by neither the stack (which
   * starts at the shrunken RAM top) nor the bootloader (above 0x2003FC08).
   */
  RAM : ORIGIN = 0x20000008, LENGTH = 255K - 256
}
